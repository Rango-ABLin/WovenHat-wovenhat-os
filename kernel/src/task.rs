use core::{
    arch::global_asm,
    cell::UnsafeCell,
    sync::atomic::{AtomicU64, Ordering},
};

use spin::Mutex;

use crate::capability::{Capability, CapabilitySet};

const MAX_TASKS: usize = 4;
const TASK_STACK_SIZE: usize = 4096 * 2;
const KERNEL_TASK_ID: TaskId = TaskId(0);
const IDLE_TASK_ID: TaskId = TaskId(1);

static SCHEDULER: Mutex<Scheduler> = Mutex::new(Scheduler::empty());
static IDLE_HEARTBEATS: AtomicU64 = AtomicU64::new(0);
static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(2);
static TASK_STACKS: [TaskStack; MAX_TASKS] = [const { TaskStack::new() }; MAX_TASKS];

global_asm!(
    ".global wovenhat_context_switch",
    "wovenhat_context_switch:",
    "push rbx",
    "push rbp",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov [rdi], rsp",
    "mov rsp, rsi",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbp",
    "pop rbx",
    "ret",
);

unsafe extern "C" {
    fn wovenhat_context_switch(previous_rsp: *mut u64, next_rsp: u64);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TaskId(u64);

impl TaskId {
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TaskState {
    Empty,
    Ready,
    Running,
    Blocked,
    Sleeping,
    Dead,
}

impl TaskState {
    const fn name(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Sleeping => "sleeping",
            Self::Dead => "dead",
        }
    }
}

#[derive(Clone, Copy)]
struct Context {
    stack_pointer: u64,
}

#[repr(align(16))]
struct TaskStack(UnsafeCell<[u8; TASK_STACK_SIZE]>);

// SAFETY: Each stack is assigned to at most one task and the bootstrap kernel
// is currently single-core. Scheduler metadata never aliases this storage.
unsafe impl Sync for TaskStack {}

impl TaskStack {
    const fn new() -> Self {
        Self(UnsafeCell::new([0; TASK_STACK_SIZE]))
    }
}

struct TaskControlBlock {
    id: TaskId,
    name: &'static str,
    state: TaskState,
    priority: u8,
    context: Context,
    capabilities: CapabilitySet,
}

impl TaskControlBlock {
    const fn empty() -> Self {
        Self {
            id: TaskId(u64::MAX),
            name: "",
            state: TaskState::Empty,
            priority: 0,
            context: Context { stack_pointer: 0 },
            capabilities: CapabilitySet::empty(),
        }
    }

    fn initialize(&mut self, slot: usize, id: TaskId, name: &'static str, entry: fn() -> !) {
        self.id = id;
        self.name = name;
        self.state = TaskState::Ready;
        self.priority = 1;

        let stack_start = TASK_STACKS[slot].0.get().cast::<u8>() as usize;
        let stack_top = stack_start + TASK_STACK_SIZE;
        let mut cursor = (stack_top & !0xf) - 8;

        // Build the stack expected by `wovenhat_context_switch`: six saved
        // callee-saved registers followed by the entry address consumed by
        // `ret`. The reserved eight bytes preserve the SysV entry alignment.
        unsafe {
            push_stack_value(&mut cursor, entry as usize as u64);
            for _ in 0..6 {
                push_stack_value(&mut cursor, 0);
            }
        }

        self.context.stack_pointer = cursor as u64;
    }
}

pub struct Summary {
    pub task_count: usize,
    pub ready_tasks: usize,
    pub blocked_tasks: usize,
    pub current_id: TaskId,
    pub current_name: &'static str,
    pub current_state: &'static str,
    pub context_switches: u64,
    pub idle_heartbeats: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CapabilityError {
    PermissionDenied,
    UnknownTask,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SpawnError {
    Full,
}

struct Scheduler {
    tasks: [TaskControlBlock; MAX_TASKS],
    current_slot: usize,
    task_count: usize,
    context_switches: u64,
    quantum: u64,
}

impl Scheduler {
    const fn empty() -> Self {
        Self {
            tasks: [const { TaskControlBlock::empty() }; MAX_TASKS],
            current_slot: 0,
            task_count: 0,
            context_switches: 0,
            quantum: 0,
        }
    }

    fn initialize(&mut self) {
        assert!(self.task_count == 0, "scheduler initialized more than once");

        self.tasks[0].id = KERNEL_TASK_ID;
        self.tasks[0].name = "kernel";
        self.tasks[0].state = TaskState::Running;
        self.tasks[0].capabilities = CapabilitySet::kernel_bootstrap();
        self.tasks[1].initialize(1, IDLE_TASK_ID, "idle", idle_task);
        self.task_count = 2;
    }

    fn prepare_switch(&mut self) -> Option<(*mut u64, u64)> {
        let next_slot = (1..=MAX_TASKS)
            .map(|offset| (self.current_slot + offset) % MAX_TASKS)
            .find(|slot| self.tasks[*slot].state == TaskState::Ready)?;

        let previous_slot = self.current_slot;
        self.tasks[previous_slot].state = TaskState::Ready;
        self.tasks[next_slot].state = TaskState::Running;
        self.current_slot = next_slot;
        self.context_switches += 1;

        let previous_rsp = &mut self.tasks[previous_slot].context.stack_pointer as *mut u64;
        let next_rsp = self.tasks[next_slot].context.stack_pointer;
        Some((previous_rsp, next_rsp))
    }

    fn summary(&self) -> Summary {
        let task = &self.tasks[self.current_slot];
        assert!(task.state == TaskState::Running);

        let ready_tasks = self.tasks.iter().filter(|task| task.state == TaskState::Ready).count();
        let blocked_tasks = self.tasks.iter().filter(|task| task.state == TaskState::Blocked).count();

        Summary {
            task_count: self.task_count,
            ready_tasks,
            blocked_tasks,
            current_id: task.id,
            current_name: task.name,
            current_state: task.state.name(),
            context_switches: self.context_switches,
            idle_heartbeats: IDLE_HEARTBEATS.load(Ordering::Relaxed),
        }
    }

    fn tick(&mut self) -> bool {
        if self.task_count < 2 {
            return false;
        }

        self.quantum += 1;
        if self.quantum < 2 {
            return false;
        }

        self.quantum = 0;
        true
    }
}

pub fn init() {
    SCHEDULER.lock().initialize();
}

pub fn spawn(name: &'static str, entry: fn() -> !) -> Result<TaskId, SpawnError> {
    let mut scheduler = SCHEDULER.lock();
    assert!(scheduler.task_count != 0, "scheduler not initialized");

    let slot = scheduler
        .tasks
        .iter()
        .position(|task| task.state == TaskState::Empty)
        .ok_or(SpawnError::Full)?;

    let id = TaskId(NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed));
    scheduler.tasks[slot].initialize(slot, id, name, entry);
    scheduler.task_count += 1;
    Ok(id)
}

pub fn yield_now() {
    let switch = {
        let mut scheduler = SCHEDULER.lock();
        assert!(scheduler.task_count != 0, "scheduler not initialized");
        scheduler.prepare_switch()
    };

    let Some((previous_rsp, next_rsp)) = switch else {
        return;
    };

    // SAFETY: Both stack pointers belong to live, statically stored task
    // control blocks. The scheduler lock was released before switching, and
    // only cooperative calls to this function can change the running task.
    unsafe { wovenhat_context_switch(previous_rsp, next_rsp) };
}

pub fn tick() {
    let should_switch = {
        let mut scheduler = SCHEDULER.lock();
        assert!(scheduler.task_count != 0, "scheduler not initialized");
        scheduler.tick()
    };

    if should_switch {
        yield_now();
    }
}

pub fn summary() -> Summary {
    let scheduler = SCHEDULER.lock();
    assert!(scheduler.task_count != 0, "scheduler not initialized");
    scheduler.summary()
}

pub fn current_has(capability: Capability) -> bool {
    let scheduler = SCHEDULER.lock();
    assert!(scheduler.task_count != 0, "scheduler not initialized");
    scheduler.tasks[scheduler.current_slot]
        .capabilities
        .contains(capability)
}

pub fn grant(target: TaskId, capability: Capability) -> Result<(), CapabilityError> {
    let mut scheduler = SCHEDULER.lock();
    assert!(scheduler.task_count != 0, "scheduler not initialized");

    let authority = scheduler.tasks[scheduler.current_slot].capabilities;
    if !authority.contains(Capability::TaskControl) || !authority.contains(capability) {
        return Err(CapabilityError::PermissionDenied);
    }

    let target_task = scheduler
        .tasks
        .iter_mut()
        .find(|task| task.state != TaskState::Empty && task.id == target)
        .ok_or(CapabilityError::UnknownTask)?;
    target_task.capabilities = target_task.capabilities.with(capability);
    Ok(())
}

pub fn revoke(target: TaskId, capability: Capability) -> Result<(), CapabilityError> {
    let mut scheduler = SCHEDULER.lock();
    assert!(scheduler.task_count != 0, "scheduler not initialized");

    if !scheduler.tasks[scheduler.current_slot]
        .capabilities
        .contains(Capability::TaskControl)
    {
        return Err(CapabilityError::PermissionDenied);
    }

    let target_task = scheduler
        .tasks
        .iter_mut()
        .find(|task| task.state != TaskState::Empty && task.id == target)
        .ok_or(CapabilityError::UnknownTask)?;
    target_task.capabilities = target_task.capabilities.without(capability);
    Ok(())
}

fn task_has(task_id: TaskId, capability: Capability) -> bool {
    SCHEDULER
        .lock()
        .tasks
        .iter()
        .find(|task| task.state != TaskState::Empty && task.id == task_id)
        .is_some_and(|task| task.capabilities.contains(capability))
}

pub fn capability_policy_valid() -> bool {
    let scheduler = SCHEDULER.lock();
    assert!(scheduler.task_count >= 2, "bootstrap tasks are missing");

    let required = [
        Capability::Console,
        Capability::TimerRead,
        Capability::TaskInspect,
        Capability::TaskControl,
        Capability::DeviceIo,
        Capability::InterruptControl,
        Capability::MemoryInspect,
    ];

    required
        .iter()
        .all(|capability| scheduler.tasks[0].capabilities.contains(*capability))
        && required
            .iter()
            .all(|capability| !scheduler.tasks[1].capabilities.contains(*capability))
}

pub fn capability_delegation_valid() -> bool {
    let granted = grant(IDLE_TASK_ID, Capability::TimerRead).is_ok();
    let observed = task_has(IDLE_TASK_ID, Capability::TimerRead);
    let revoked = revoke(IDLE_TASK_ID, Capability::TimerRead).is_ok();
    let denied_after_revoke = !task_has(IDLE_TASK_ID, Capability::TimerRead);

    granted && observed && revoked && denied_after_revoke
}

fn idle_task() -> ! {
    loop {
        IDLE_HEARTBEATS.fetch_add(1, Ordering::Relaxed);
        yield_now();
    }
}

/// Push a value onto a task stack being prepared for its first context switch.
///
/// # Safety
///
/// `cursor` must point within a writable, properly aligned task stack and have
/// room for another eight-byte value.
unsafe fn push_stack_value(cursor: &mut usize, value: u64) {
    *cursor -= size_of::<u64>();

    // SAFETY: The caller guarantees that the decremented cursor is a writable,
    // aligned location within the task's private stack.
    unsafe { (*cursor as *mut u64).write(value) };
}
