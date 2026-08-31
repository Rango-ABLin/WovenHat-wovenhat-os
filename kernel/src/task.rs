use core::{
    arch::global_asm,
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use spin::Mutex;

use crate::{
    capability::{Capability, CapabilitySet},
    gdt,
};

const MAX_TASKS: usize = 4;
const TASK_STACK_SIZE: usize = 4096 * 2;
const KERNEL_TASK_ID: TaskId = TaskId(0);
const IDLE_TASK_ID: TaskId = TaskId(1);

static SCHEDULER: Mutex<Scheduler> = Mutex::new(Scheduler::empty());
static IDLE_HEARTBEATS: AtomicU64 = AtomicU64::new(0);
static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(2);
static PREEMPTION_TICKS: AtomicU64 = AtomicU64::new(0);
static PREEMPTION_REQUESTED: AtomicBool = AtomicBool::new(false);
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

global_asm!(
    ".global wovenhat_enter_user_mode",
    "wovenhat_enter_user_mode:",
    "mov ax, cx",
    "mov ds, ax",
    "mov es, ax",
    "mov fs, ax",
    "mov gs, ax",
    "mov ss, ax",
    "push rcx",
    "push rsi",
    "pushfq",
    "or qword ptr [rsp], 0x200",
    "push rdx",
    "push rdi",
    "iretq",
);

unsafe extern "C" {
    fn wovenhat_context_switch(previous_rsp: *mut u64, next_rsp: u64);
    fn wovenhat_enter_user_mode(entry: u64, stack_top: u64, user_cs: u16, user_ss: u16) -> !;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TaskId(u64);

impl TaskId {
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ProcessId(u64);

impl ProcessId {
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    New,
    Ready,
    Running,
    Blocked,
    Exited,
}

impl ProcessState {
    pub const fn name(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Exited => "exited",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaskPriority(u8);

impl TaskPriority {
    pub const LOW: Self = Self(0);
    pub const NORMAL: Self = Self(1);
    pub const HIGH: Self = Self(2);

    pub const fn as_u8(self) -> u8 {
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
    priority: TaskPriority,
    context: Context,
    capabilities: CapabilitySet,
}

impl TaskControlBlock {
    const fn empty() -> Self {
        Self {
            id: TaskId(u64::MAX),
            name: "",
            state: TaskState::Empty,
            priority: TaskPriority::NORMAL,
            context: Context { stack_pointer: 0 },
            capabilities: CapabilitySet::empty(),
        }
    }

    fn initialize(&mut self, slot: usize, id: TaskId, name: &'static str, entry: fn() -> !, priority: TaskPriority) {
        self.id = id;
        self.name = name;
        self.state = TaskState::Ready;
        self.priority = priority;

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

pub struct Process {
    pub id: ProcessId,
    pub parent: Option<ProcessId>,
    pub task_id: TaskId,
    pub name: &'static str,
    pub state: ProcessState,
    pub ring: u8,
    pub entry: u64,
    pub stack_top: u64,
    pub stack_size: u64,
}

pub struct Summary {
    pub task_count: usize,
    pub ready_tasks: usize,
    pub blocked_tasks: usize,
    pub current_id: TaskId,
    pub current_name: &'static str,
    pub current_state: &'static str,
    pub current_priority: u8,
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
}

impl Scheduler {
    const fn empty() -> Self {
        Self {
            tasks: [const { TaskControlBlock::empty() }; MAX_TASKS],
            current_slot: 0,
            task_count: 0,
            context_switches: 0,
        }
    }

    fn initialize(&mut self) {
        assert!(self.task_count == 0, "scheduler initialized more than once");

        self.tasks[0].id = KERNEL_TASK_ID;
        self.tasks[0].name = "kernel";
        self.tasks[0].state = TaskState::Running;
        self.tasks[0].priority = TaskPriority::HIGH;
        self.tasks[0].capabilities = CapabilitySet::kernel_bootstrap();
        self.tasks[1].initialize(1, IDLE_TASK_ID, "idle", idle_task, TaskPriority::LOW);
        self.task_count = 2;
    }

    fn prepare_switch(&mut self) -> Option<(*mut u64, u64)> {
        let mut best_slot = None;
        let mut best_priority = TaskPriority::LOW;

        for offset in 0..MAX_TASKS {
            let slot = (self.current_slot + offset) % MAX_TASKS;
            let task = &self.tasks[slot];
            if task.state != TaskState::Ready {
                continue;
            }

            if task.priority >= best_priority {
                best_priority = task.priority;
                best_slot = Some(slot);
            }
        }

        let next_slot = best_slot?;
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
            current_priority: task.priority.as_u8(),
            context_switches: self.context_switches,
            idle_heartbeats: IDLE_HEARTBEATS.load(Ordering::Relaxed),
        }
    }

}

static NEXT_PROCESS_ID: AtomicU64 = AtomicU64::new(1);

pub fn init() {
    SCHEDULER.lock().initialize();
}

pub fn create_process(name: &'static str, entry: usize, stack_top: usize, ring: u8) -> Process {
    let task_id = SCHEDULER
        .lock()
        .tasks
        .iter()
        .find(|task| task.state == TaskState::Empty)
        .map(|_| TaskId(NEXT_TASK_ID.load(Ordering::Relaxed)))
        .unwrap_or(TaskId(0));

    let id = ProcessId(NEXT_PROCESS_ID.fetch_add(1, Ordering::Relaxed));
    Process {
        id,
        parent: None,
        task_id,
        name,
        state: ProcessState::Ready,
        ring,
        entry: entry as u64,
        stack_top: stack_top as u64,
        stack_size: 4096,
    }
}

pub fn current_process() -> Option<Process> {
    let scheduler = SCHEDULER.lock();
    let task = &scheduler.tasks[scheduler.current_slot];
    if task.state == TaskState::Empty {
        return None;
    }

    Some(Process {
        id: ProcessId(0),
        parent: None,
        task_id: task.id,
        name: task.name,
        state: ProcessState::Running,
        ring: 0,
        entry: 0,
        stack_top: task.context.stack_pointer,
        stack_size: TASK_STACK_SIZE as u64,
    })
}

#[derive(Clone, Copy, Debug)]
pub struct UserTaskContext {
    pub entry: u64,
    pub stack_top: u64,
    pub code_segment: u16,
    pub data_segment: u16,
}

pub fn prepare_user_context(entry: usize, stack_top: usize) -> UserTaskContext {
    let (code_segment, data_segment) = gdt::user_segments();
    UserTaskContext {
        entry: entry as u64,
        stack_top: stack_top as u64,
        code_segment: code_segment.0,
        data_segment: data_segment.0,
    }
}

pub fn enter_user_mode(entry: usize, stack_top: usize) -> ! {
    let context = prepare_user_context(entry, stack_top);
    unsafe {
        wovenhat_enter_user_mode(
            context.entry,
            context.stack_top,
            context.code_segment,
            context.data_segment,
        )
    }
}

pub fn spawn(name: &'static str, entry: fn() -> !) -> Result<TaskId, SpawnError> {
    spawn_with_priority(name, entry, TaskPriority::NORMAL)
}

pub fn spawn_with_priority(
    name: &'static str,
    entry: fn() -> !,
    priority: TaskPriority,
) -> Result<TaskId, SpawnError> {
    let mut scheduler = SCHEDULER.lock();
    assert!(scheduler.task_count != 0, "scheduler not initialized");

    let slot = scheduler
        .tasks
        .iter()
        .position(|task| task.state == TaskState::Empty)
        .ok_or(SpawnError::Full)?;

    let id = TaskId(NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed));
    scheduler.tasks[slot].initialize(slot, id, name, entry, priority);
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
    if PREEMPTION_TICKS.fetch_add(1, Ordering::Relaxed) + 1 >= 2 {
        PREEMPTION_TICKS.store(0, Ordering::Relaxed);
        PREEMPTION_REQUESTED.store(true, Ordering::Release);
    }
}

pub fn preemption_point() {
    if PREEMPTION_REQUESTED.swap(false, Ordering::AcqRel) {
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
        preemption_point();
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
