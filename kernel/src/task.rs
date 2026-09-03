use core::{
    arch::global_asm,
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use spin::Mutex;

use crate::{
    capability::{Capability, CapabilitySet},
    config::{MAX_FILE_DESCRIPTORS, MAX_PROCESSES, MAX_TASKS, TASK_STACK_SIZE},
    gdt, ipc, paging, timer, userspace, vfs,
};

const KERNEL_TASK_ID: TaskId = TaskId(0);
const IDLE_TASK_ID: TaskId = TaskId(1);

static SCHEDULER: Mutex<Scheduler> = Mutex::new(Scheduler::empty());
static PROCESS_TABLE: Mutex<[Option<Process>; MAX_PROCESSES]> =
    Mutex::new([const { None }; MAX_PROCESSES]);
static IDLE_HEARTBEATS: AtomicU64 = AtomicU64::new(0);
static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(2);
static NEXT_PROCESS_ID: AtomicU64 = AtomicU64::new(1);
static PREEMPTION_REQUESTED: AtomicBool = AtomicBool::new(false);
static PREEMPTION_SWITCHES: AtomicU64 = AtomicU64::new(0);
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
    "cli",
    "movzx ecx, cx",
    "movzx edx, dx",
    "push rcx",
    "push rsi",
    "pushfq",
    "or qword ptr [rsp], 0x200",
    "push rdx",
    "push rdi",
    "iretq",
);

global_asm!(
    ".global wovenhat_user_stub",
    "wovenhat_user_stub:",
    "mov rax, 3",
    "int 0x80",
    "2:",
    "jmp 2b",
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
pub struct Credentials {
    pub uid: u32,
    pub gid: u32,
}

impl Credentials {
    pub const ROOT: Self = Self { uid: 0, gid: 0 };
    pub const USERSPACE: Self = Self {
        uid: 1000,
        gid: 1000,
    };

    pub const fn is_root(self) -> bool {
        self.uid == 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Ready,
    Exited,
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

    const fn index(self) -> usize {
        self.0 as usize
    }

    const fn quantum(self) -> u8 {
        match self {
            Self::LOW => 1,
            Self::NORMAL => 2,
            Self::HIGH => 4,
            _ => 1,
        }
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
    entry: Option<fn() -> !>,
    wake_tick: u64,
    user_context: Option<UserTaskContext>,
    address_space: Option<paging::AddressSpace>,
    remaining_ticks: u8,
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
            entry: None,
            wake_tick: 0,
            user_context: None,
            address_space: None,
            remaining_ticks: 0,
        }
    }

    fn initialize(
        &mut self,
        slot: usize,
        id: TaskId,
        name: &'static str,
        entry: fn() -> !,
        priority: TaskPriority,
    ) {
        self.id = id;
        self.name = name;
        self.state = TaskState::Ready;
        self.priority = priority;
        self.entry = Some(entry);
        self.wake_tick = 0;
        self.user_context = None;
        self.address_space = paging::kernel_address_space();
        self.remaining_ticks = priority.quantum();

        let stack_start = TASK_STACKS[slot].0.get().cast::<u8>() as usize;
        let stack_top = stack_start + TASK_STACK_SIZE;
        let mut cursor = (stack_top & !0xf) - 8;

        // Build the stack expected by `wovenhat_context_switch`: six saved
        // callee-saved registers followed by the entry address consumed by
        // `ret`. The reserved eight bytes preserve the SysV entry alignment.
        unsafe {
            push_stack_value(&mut cursor, (task_bootstrap as fn() -> !) as usize as u64);
            for _ in 0..6 {
                push_stack_value(&mut cursor, 0);
            }
        }

        self.context.stack_pointer = cursor as u64;
    }

    fn initialize_user(
        &mut self,
        slot: usize,
        id: TaskId,
        name: &'static str,
        context: UserTaskContext,
        address_space: paging::AddressSpace,
    ) {
        self.id = id;
        self.name = name;
        self.state = TaskState::Ready;
        self.priority = TaskPriority::NORMAL;
        self.entry = None;
        self.wake_tick = 0;
        self.user_context = Some(context);
        self.address_space = Some(address_space);
        self.remaining_ticks = TaskPriority::NORMAL.quantum();
        self.capabilities = CapabilitySet::userspace();

        let stack_start = TASK_STACKS[slot].0.get().cast::<u8>() as usize;
        let stack_top = stack_start + TASK_STACK_SIZE;
        let mut cursor = (stack_top & !0xf) - 8;

        // The first kernel-side dispatch enters through the common bootstrap;
        // it then installs the saved user context with iretq.
        unsafe {
            push_stack_value(&mut cursor, (task_bootstrap as fn() -> !) as usize as u64);
            for _ in 0..6 {
                push_stack_value(&mut cursor, 0);
            }
        }

        self.context.stack_pointer = cursor as u64;
    }
    fn initialize_fork(
        &mut self,
        slot: usize,
        id: TaskId,
        frame: crate::syscall::UserForkFrame,
        address_space: paging::AddressSpace,
        capabilities: CapabilitySet,
    ) {
        self.id = id;
        self.name = "fork-child";
        self.state = TaskState::Ready;
        self.priority = TaskPriority::NORMAL;
        self.entry = None;
        self.wake_tick = 0;
        self.user_context = None;
        self.address_space = Some(address_space);
        self.remaining_ticks = TaskPriority::NORMAL.quantum();
        self.capabilities = capabilities;

        let stack_start = TASK_STACKS[slot].0.get().cast::<u8>() as usize;
        let stack_top = (stack_start + TASK_STACK_SIZE) & !0xf;
        let frame_start = stack_top - core::mem::size_of::<crate::syscall::UserForkFrame>();
        unsafe {
            (frame_start as *mut crate::syscall::UserForkFrame).write(frame.child_return());
        }
        let mut cursor = frame_start;
        unsafe {
            push_stack_value(&mut cursor, crate::syscall::resume_address());
            for _ in 0..6 {
                push_stack_value(&mut cursor, 0);
            }
        }
        self.context.stack_pointer = cursor as u64;
    }
}

fn task_bootstrap() -> ! {
    let (entry, user_context) = {
        let scheduler = SCHEDULER.lock();
        let task = &scheduler.tasks[scheduler.current_slot];
        (task.entry, task.user_context)
    };

    x86_64::instructions::interrupts::enable();
    if let Some(context) = user_context {
        enter_user_context(context)
    }

    entry.expect("scheduled task is missing its entry point")()
}

#[derive(Clone, Copy)]
/// What a process file descriptor refers to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FdKind {
    File(vfs::OpenFileId),
    PipeRead(usize),
    PipeWrite(usize),
}

pub struct Process {
    pub id: ProcessId,
    pub task_id: TaskId,
    pub state: ProcessState,
    pub parent: ProcessId,
    pub credentials: Credentials,
    pub exit_code: i32,
    address_space: Option<userspace::AddressSpace>,
    /// Per-process file-descriptor table. Entries are handles into the
    /// system-wide refcounted open-file table in `vfs`.
    files: [Option<FdKind>; MAX_FILE_DESCRIPTORS],
    memory_mappings: [Option<userspace::AnonymousMapping>; userspace::MAX_ANONYMOUS_MAPPINGS],
    cwd: [u8; crate::config::MAX_PATH_SIZE],
    cwd_len: usize,
    pending_signal: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProcessError {
    Full,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WaitError {
    NoSuchChild,
    StillRunning,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FileError {
    PermissionDenied,
    NoProcess,
    NotFound,
    TooManyFiles,
    BadDescriptor,
    AlreadyExists,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MemoryError {
    NoProcess,
    InvalidLength,
    Full,
    NotFound,
    MappingFailed,
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
    pub preemption_switches: u64,
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

struct ContextSwitch {
    previous_rsp: *mut u64,
    next_rsp: u64,
    next_address_space: paging::AddressSpace,
}
struct Scheduler {
    tasks: [TaskControlBlock; MAX_TASKS],
    current_slot: usize,
    task_count: usize,
    context_switches: u64,
    last_selected: [usize; 3],
}

impl Scheduler {
    const fn empty() -> Self {
        Self {
            tasks: [const { TaskControlBlock::empty() }; MAX_TASKS],
            current_slot: 0,
            task_count: 0,
            context_switches: 0,
            last_selected: [0; 3],
        }
    }

    fn initialize(&mut self) {
        assert!(self.task_count == 0, "scheduler initialized more than once");

        self.tasks[0].id = KERNEL_TASK_ID;
        self.tasks[0].name = "kernel";
        self.tasks[0].state = TaskState::Running;
        self.tasks[0].priority = TaskPriority::HIGH;
        self.tasks[0].capabilities = CapabilitySet::kernel_bootstrap();
        self.tasks[0].address_space = paging::kernel_address_space();
        self.tasks[0].remaining_ticks = TaskPriority::HIGH.quantum();
        self.tasks[1].initialize(1, IDLE_TASK_ID, "idle", idle_task, TaskPriority::LOW);
        self.task_count = 2;
    }

    fn prepare_switch(&mut self) -> Option<ContextSwitch> {
        self.reap_dead();

        let best_priority = self
            .tasks
            .iter()
            .filter(|task| task.state == TaskState::Ready)
            .map(|task| task.priority)
            .max()?;
        let priority_index = best_priority.index();
        let start = self.last_selected[priority_index];
        let next_slot = (1..=MAX_TASKS)
            .map(|offset| (start + offset) % MAX_TASKS)
            .find(|slot| {
                let task = &self.tasks[*slot];
                task.state == TaskState::Ready && task.priority == best_priority
            })?;
        self.last_selected[priority_index] = next_slot;

        let previous_slot = self.current_slot;
        if self.tasks[previous_slot].state == TaskState::Running {
            self.tasks[previous_slot].state = TaskState::Ready;
        }
        self.tasks[next_slot].state = TaskState::Running;
        self.tasks[next_slot].remaining_ticks = self.tasks[next_slot].priority.quantum();
        self.current_slot = next_slot;
        self.context_switches += 1;

        let previous_rsp = &mut self.tasks[previous_slot].context.stack_pointer as *mut u64;
        let next_rsp = self.tasks[next_slot].context.stack_pointer;
        let next_address_space = self.tasks[next_slot].address_space?;
        Some(ContextSwitch {
            previous_rsp,
            next_rsp,
            next_address_space,
        })
    }
    fn reap_dead(&mut self) {
        for (slot, task) in self.tasks.iter_mut().enumerate() {
            if slot != self.current_slot && task.state == TaskState::Dead {
                *task = TaskControlBlock::empty();
            }
        }
    }

    fn wake_sleeping(&mut self, now: u64) {
        for task in &mut self.tasks {
            if task.state == TaskState::Sleeping && task.wake_tick <= now {
                task.state = TaskState::Ready;
                task.wake_tick = 0;
            }
        }
    }

    fn summary(&self) -> Summary {
        let task = &self.tasks[self.current_slot];
        assert!(task.state == TaskState::Running);

        let ready_tasks = self
            .tasks
            .iter()
            .filter(|task| task.state == TaskState::Ready)
            .count();
        let blocked_tasks = self
            .tasks
            .iter()
            .filter(|task| matches!(task.state, TaskState::Blocked | TaskState::Sleeping))
            .count();

        Summary {
            task_count: self.task_count,
            ready_tasks,
            blocked_tasks,
            current_id: task.id,
            current_name: task.name,
            current_state: task.state.name(),
            current_priority: task.priority.as_u8(),
            context_switches: self.context_switches,
            preemption_switches: PREEMPTION_SWITCHES.load(Ordering::Relaxed),
            idle_heartbeats: IDLE_HEARTBEATS.load(Ordering::Relaxed),
        }
    }
}

unsafe fn switch_stacks(context_switch: ContextSwitch) {
    paging::switch_to(context_switch.next_address_space);
    unsafe {
        wovenhat_context_switch(context_switch.previous_rsp, context_switch.next_rsp);
    }
}
pub fn init() {
    SCHEDULER.lock().initialize();
}

fn create_process(
    task_id: TaskId,
    parent: ProcessId,
    address_space: userspace::AddressSpace,
) -> Process {
    let mut cwd = [0u8; crate::config::MAX_PATH_SIZE];
    cwd[0] = b'/';
    let mut cwd_len = 1usize;
    {
        let processes = PROCESS_TABLE.lock();
        if let Some(parent_proc) = processes.iter().flatten().find(|p| p.id == parent) {
            cwd = parent_proc.cwd;
            cwd_len = parent_proc.cwd_len;
        }
    }
    Process {
        id: ProcessId(NEXT_PROCESS_ID.fetch_add(1, Ordering::Relaxed)),
        task_id,
        state: ProcessState::Ready,
        parent,
        credentials: Credentials::USERSPACE,
        exit_code: 0,
        address_space: Some(address_space),
        files: [None; MAX_FILE_DESCRIPTORS],
        memory_mappings: [None; userspace::MAX_ANONYMOUS_MAPPINGS],
        cwd,
        cwd_len,
        pending_signal: 0,
    }
}

/// Clone a parent's file-descriptor table for fork.
/// Each live descriptor bumps the shared open-file refcount so offsets are shared.
fn clone_file_table(
    parent_files: &[Option<FdKind>; MAX_FILE_DESCRIPTORS],
) -> Result<[Option<FdKind>; MAX_FILE_DESCRIPTORS], ProcessError> {
    let mut child_files = [None; MAX_FILE_DESCRIPTORS];
    for (index, entry) in parent_files.iter().enumerate() {
        match entry {
            Some(FdKind::File(id)) => {
                child_files[index] =
                    Some(FdKind::File(vfs::clone_open_file(*id).map_err(|_| ProcessError::Full)?));
            }
            Some(FdKind::PipeRead(id)) => {
                crate::pipe::clone_reader(*id).map_err(|_| ProcessError::Full)?;
                child_files[index] = Some(FdKind::PipeRead(*id));
            }
            Some(FdKind::PipeWrite(id)) => {
                crate::pipe::clone_writer(*id).map_err(|_| ProcessError::Full)?;
                child_files[index] = Some(FdKind::PipeWrite(*id));
            }
            None => {}
        }
    }
    Ok(child_files)
}

fn release_fd(fd: FdKind) {
    match fd {
        FdKind::File(id) => {
            let _ = vfs::close_open_file(id);
        }
        FdKind::PipeRead(id) => crate::pipe::close_reader(id),
        FdKind::PipeWrite(id) => crate::pipe::close_writer(id),
    }
}

fn release_file_table(files: &mut [Option<FdKind>; MAX_FILE_DESCRIPTORS]) {
    for entry in files.iter_mut() {
        if let Some(fd) = entry.take() {
            release_fd(fd);
        }
    }
}

pub fn spawn_user_process(
    name: &'static str,
    program: userspace::UserProgram,
) -> Result<(ProcessId, UserTaskContext), ProcessError> {
    if !program.image.is_valid() {
        let _ = userspace::destroy(program.address_space);
        return Err(ProcessError::Full);
    }

    let parent = ProcessId(current_process_id());
    let context = prepare_user_context(program.image.entry as usize, program.stack.top as usize);
    let mut scheduler = SCHEDULER.lock();
    scheduler.reap_dead();
    let Some(task_slot) = scheduler
        .tasks
        .iter()
        .position(|task| task.state == TaskState::Empty)
    else {
        let _ = userspace::destroy(program.address_space);
        return Err(ProcessError::Full);
    };

    let mut processes = PROCESS_TABLE.lock();
    let Some(process_slot) = processes.iter().position(|entry| entry.is_none()) else {
        let _ = userspace::destroy(program.address_space);
        return Err(ProcessError::Full);
    };

    let task_id = TaskId(NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed));
    let process = create_process(task_id, parent, program.address_space);
    let process_id = process.id;
    if ipc::register(process_id.as_u64()).is_err() {
        let _ = userspace::destroy(program.address_space);
        return Err(ProcessError::Full);
    }
    scheduler.tasks[task_slot].initialize_user(
        task_slot,
        task_id,
        name,
        context,
        program.address_space.paging(),
    );
    scheduler.task_count += 1;
    processes[process_slot] = Some(process);
    Ok((process_id, context))
}
pub fn process_exited(id: ProcessId) -> bool {
    PROCESS_TABLE
        .lock()
        .iter()
        .flatten()
        .find(|process| process.id == id)
        .is_some_and(|process| process.state == ProcessState::Exited)
}

pub fn current_process_id() -> u64 {
    let task_id = current_task_id();
    PROCESS_TABLE
        .lock()
        .iter()
        .flatten()
        .find(|process| process.task_id == task_id)
        .map_or(task_id.as_u64(), |process| process.id.as_u64())
}

/// Attempt to resolve a user write fault via copy-on-write.
/// Returns true if the fault was handled and the process may resume.
pub fn try_handle_cow_fault(fault_address: u64) -> bool {
    let task_id = current_task_id();
    let processes = PROCESS_TABLE.lock();
    let Some(process) = processes
        .iter()
        .flatten()
        .find(|process| process.task_id == task_id)
    else {
        return false;
    };
    let Some(address_space) = process.address_space else {
        return false;
    };
    if !address_space.is_logically_writable(fault_address, &process.memory_mappings) {
        return false;
    }
    let paging_as = address_space.paging();
    drop(processes);
    paging::try_break_cow(paging_as, fault_address)
}

pub fn current_credentials() -> Credentials {
    let task_id = current_task_id();
    PROCESS_TABLE
        .lock()
        .iter()
        .flatten()
        .find(|process| process.task_id == task_id)
        .map_or(Credentials::ROOT, |process| process.credentials)
}

pub fn process_credentials(id: ProcessId) -> Option<Credentials> {
    PROCESS_TABLE
        .lock()
        .iter()
        .flatten()
        .find(|process| process.id == id)
        .map(|process| process.credentials)
}

pub fn may_ipc_with(receiver: u64) -> bool {
    let sender = current_credentials();
    let Some(receiver) = process_credentials(ProcessId(receiver)) else {
        return false;
    };
    credentials_may_ipc(sender, receiver)
}

pub fn credential_policy_valid() -> bool {
    let peer = Credentials {
        uid: Credentials::USERSPACE.uid,
        gid: 2000,
    };
    let group_peer = Credentials {
        uid: 2000,
        gid: Credentials::USERSPACE.gid,
    };
    let stranger = Credentials {
        uid: 2000,
        gid: 2000,
    };
    Credentials::ROOT.is_root()
        && !Credentials::USERSPACE.is_root()
        && Credentials::ROOT != Credentials::USERSPACE
        && credentials_may_ipc(Credentials::ROOT, stranger)
        && credentials_may_ipc(Credentials::USERSPACE, peer)
        && credentials_may_ipc(Credentials::USERSPACE, group_peer)
        && !credentials_may_ipc(Credentials::USERSPACE, stranger)
}

fn credentials_may_ipc(sender: Credentials, receiver: Credentials) -> bool {
    sender.is_root() || sender.uid == receiver.uid || sender.gid == receiver.gid
}
pub fn fork_current(frame: crate::syscall::UserForkFrame) -> Result<ProcessId, ProcessError> {
    let task_id = current_task_id();
    let (parent, capabilities) = {
        let scheduler = SCHEDULER.lock();
        let capabilities = scheduler.tasks[scheduler.current_slot].capabilities;
        let processes = PROCESS_TABLE.lock();
        let parent = processes
            .iter()
            .flatten()
            .find(|process| process.task_id == task_id)
            .copied()
            .ok_or(ProcessError::Full)?;
        (parent, capabilities)
    };
    let source_address_space = parent.address_space.ok_or(ProcessError::Full)?;
    let cloned_address_space =
        userspace::clone_address_space(source_address_space, &parent.memory_mappings)
            .ok_or(ProcessError::Full)?;

    let mut scheduler = SCHEDULER.lock();
    scheduler.reap_dead();
    let Some(task_slot) = scheduler
        .tasks
        .iter()
        .position(|task| task.state == TaskState::Empty)
    else {
        let _ =
            userspace::destroy_process_address_space(cloned_address_space, parent.memory_mappings);
        return Err(ProcessError::Full);
    };
    let mut processes = PROCESS_TABLE.lock();
    let Some(process_slot) = processes.iter().position(Option::is_none) else {
        drop(processes);
        drop(scheduler);
        let _ =
            userspace::destroy_process_address_space(cloned_address_space, parent.memory_mappings);
        return Err(ProcessError::Full);
    };
    let child_task_id = TaskId(NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed));
    let child_id = ProcessId(NEXT_PROCESS_ID.fetch_add(1, Ordering::Relaxed));
    if ipc::register(child_id.as_u64()).is_err() {
        drop(processes);
        drop(scheduler);
        let _ =
            userspace::destroy_process_address_space(cloned_address_space, parent.memory_mappings);
        return Err(ProcessError::Full);
    }
    let child_files = match clone_file_table(&parent.files) {
        Ok(files) => files,
        Err(err) => {
            drop(processes);
            drop(scheduler);
            let _ = userspace::destroy_process_address_space(
                cloned_address_space,
                parent.memory_mappings,
            );
            return Err(err);
        }
    };
    scheduler.tasks[task_slot].initialize_fork(
        task_slot,
        child_task_id,
        frame,
        cloned_address_space.paging(),
        capabilities,
    );
    scheduler.task_count += 1;
    processes[process_slot] = Some(Process {
        id: child_id,
        task_id: child_task_id,
        state: ProcessState::Ready,
        parent: parent.id,
        credentials: parent.credentials,
        exit_code: 0,
        address_space: Some(cloned_address_space),
        files: child_files,
        memory_mappings: parent.memory_mappings,
        cwd: parent.cwd,
        cwd_len: parent.cwd_len,
        pending_signal: 0,
    });
    Ok(child_id)
}
pub fn exec_current(program: userspace::UserProgram) -> ! {
    assert!(program.image.is_valid(), "exec received an invalid image");
    x86_64::instructions::interrupts::disable();
    let context = prepare_user_context(program.image.entry as usize, program.stack.top as usize);
    let new_address_space = program.address_space;
    let task_id = current_task_id();
    let (old_address_space, old_mappings) = {
        let mut scheduler = SCHEDULER.lock();
        let slot = scheduler.current_slot;
        assert!(scheduler.tasks[slot].id == task_id);

        let mut processes = PROCESS_TABLE.lock();
        let process = processes
            .iter_mut()
            .flatten()
            .find(|process| process.task_id == task_id)
            .expect("exec process is missing from the process table");
        let old_address_space = process
            .address_space
            .replace(new_address_space)
            .expect("exec process has no owned address space");
        let old_mappings = core::mem::replace(
            &mut process.memory_mappings,
            [None; userspace::MAX_ANONYMOUS_MAPPINGS],
        );

        scheduler.tasks[slot].user_context = Some(context);
        scheduler.tasks[slot].address_space = Some(new_address_space.paging());
        (old_address_space, old_mappings)
    };

    paging::switch_to(new_address_space.paging());
    for mapping in old_mappings.into_iter().flatten() {
        assert!(
            userspace::unmap_anonymous(old_address_space, mapping),
            "failed to release an exec mapping"
        );
    }
    assert!(
        userspace::destroy(old_address_space),
        "failed to release the replaced exec image"
    );
    x86_64::instructions::interrupts::enable();
    enter_user_context(context)
}
pub fn exit_current_process(exit_code: i32) -> ! {
    x86_64::instructions::interrupts::disable();
    let (context_switch, address_space, memory_mappings) = {
        let mut scheduler = SCHEDULER.lock();
        let slot = scheduler.current_slot;
        let task_id = scheduler.tasks[slot].id;
        assert!(
            task_id != KERNEL_TASK_ID,
            "kernel task cannot exit as a process"
        );

        let (address_space, memory_mappings) = PROCESS_TABLE
            .lock()
            .iter_mut()
            .flatten()
            .find(|process| process.task_id == task_id)
            .and_then(|process| {
                process.state = ProcessState::Exited;
                process.exit_code = exit_code;
                // Drop open-file references as soon as the process exits so
                // offsets and description slots are not held by zombies.
                release_file_table(&mut process.files);
                process.address_space.take().map(|address_space| {
                    let memory_mappings = core::mem::replace(
                        &mut process.memory_mappings,
                        [None; userspace::MAX_ANONYMOUS_MAPPINGS],
                    );
                    (address_space, memory_mappings)
                })
            })
            .expect("exiting process has no owned address space");

        scheduler.tasks[slot].state = TaskState::Dead;
        scheduler.tasks[slot].name = "exited";
        scheduler.task_count = scheduler.task_count.saturating_sub(1);
        (scheduler.prepare_switch(), address_space, memory_mappings)
    };

    if let Some(context_switch) = context_switch {
        paging::switch_to(context_switch.next_address_space);
        for mapping in memory_mappings.into_iter().flatten() {
            assert!(
                userspace::unmap_anonymous(address_space, mapping),
                "failed to release anonymous process mapping"
            );
        }
        assert!(
            userspace::destroy(address_space),
            "failed to release exiting process address space"
        );
        unsafe { switch_stacks(context_switch) };
    }

    loop {
        x86_64::instructions::hlt();
    }
}
pub fn wait_process(child_id: u64) -> Result<i32, WaitError> {
    let parent = ProcessId(current_process_id());
    let child = ProcessId(child_id);
    let mut processes = PROCESS_TABLE.lock();
    let Some(slot) = processes.iter().position(
        |entry| matches!(entry, Some(process) if process.id == child && process.parent == parent),
    ) else {
        return Err(WaitError::NoSuchChild);
    };
    let process = processes[slot].as_mut().ok_or(WaitError::NoSuchChild)?;
    if process.state != ProcessState::Exited {
        return Err(WaitError::StillRunning);
    }
    // Defensive: ensure no open-file references remain when the process slot is freed.
    release_file_table(&mut process.files);
    let exit_code = process.exit_code;
    assert!(
        ipc::unregister(child.as_u64()).is_ok(),
        "reaped process has no IPC endpoint"
    );
    processes[slot] = None;
    Ok(exit_code)
}

pub fn zombie_count() -> usize {
    PROCESS_TABLE
        .lock()
        .iter()
        .flatten()
        .filter(|process| process.state == ProcessState::Exited)
        .count()
}
pub fn mmap_current(length: u64, writable: bool) -> Result<u64, MemoryError> {
    let length = usize::try_from(length).map_err(|_| MemoryError::InvalidLength)?;
    let size = length
        .checked_add(4095)
        .map(|size| size & !4095)
        .filter(|size| *size != 0)
        .ok_or(MemoryError::InvalidLength)?;
    let task_id = current_task_id();
    let (process_index, slot, address_space) = {
        let processes = PROCESS_TABLE.lock();
        let process_index = processes
            .iter()
            .position(|process| process.is_some_and(|process| process.task_id == task_id))
            .ok_or(MemoryError::NoProcess)?;
        let process = processes[process_index].ok_or(MemoryError::NoProcess)?;
        let slot = process
            .memory_mappings
            .iter()
            .position(Option::is_none)
            .ok_or(MemoryError::Full)?;
        let address_space = process.address_space.ok_or(MemoryError::NoProcess)?;
        (process_index, slot, address_space)
    };
    let mapping = userspace::map_anonymous(address_space, slot, size, writable)
        .ok_or(MemoryError::MappingFailed)?;
    let mut processes = PROCESS_TABLE.lock();
    let process = processes[process_index]
        .as_mut()
        .ok_or(MemoryError::NoProcess)?;
    if process.memory_mappings[slot].is_some() {
        let _ = userspace::unmap_anonymous(address_space, mapping);
        return Err(MemoryError::Full);
    }
    process.memory_mappings[slot] = Some(mapping);
    Ok(mapping.address)
}

pub fn munmap_current(address: u64, length: u64) -> Result<(), MemoryError> {
    let length = usize::try_from(length).map_err(|_| MemoryError::InvalidLength)?;
    let size = length
        .checked_add(4095)
        .map(|size| size & !4095)
        .filter(|size| *size != 0)
        .ok_or(MemoryError::InvalidLength)?;
    let task_id = current_task_id();
    let (process_index, slot, address_space, mapping) = {
        let processes = PROCESS_TABLE.lock();
        let process_index = processes
            .iter()
            .position(|process| process.is_some_and(|process| process.task_id == task_id))
            .ok_or(MemoryError::NoProcess)?;
        let process = processes[process_index].ok_or(MemoryError::NoProcess)?;
        let slot = process
            .memory_mappings
            .iter()
            .position(|mapping| {
                mapping.is_some_and(|mapping| mapping.address == address && mapping.size == size)
            })
            .ok_or(MemoryError::NotFound)?;
        let mapping = process.memory_mappings[slot].ok_or(MemoryError::NotFound)?;
        let address_space = process.address_space.ok_or(MemoryError::NoProcess)?;
        (process_index, slot, address_space, mapping)
    };
    if !userspace::unmap_anonymous(address_space, mapping) {
        return Err(MemoryError::MappingFailed);
    }
    PROCESS_TABLE.lock()[process_index]
        .as_mut()
        .ok_or(MemoryError::NoProcess)?
        .memory_mappings[slot] = None;
    Ok(())
}

pub fn anonymous_mapping_count() -> usize {
    PROCESS_TABLE
        .lock()
        .iter()
        .flatten()
        .map(|process| process.memory_mappings.iter().flatten().count())
        .sum()
}
pub fn open_current(path: &str) -> Result<u64, FileError> {
    if !current_has(Capability::FileRead) {
        return Err(FileError::PermissionDenied);
    }
    let file = match vfs::open(path) {
        Ok(id) => id,
        Err(_) => {
            // POSIX-ish O_CREAT for missing files when writer-capable.
            if !current_has(Capability::FileWrite) {
                return Err(FileError::NotFound);
            }
            vfs::write_file(path, &[]).map_err(|_| FileError::NotFound)?;
            vfs::open(path).map_err(|_| FileError::NotFound)?
        }
    };
    let task_id = current_task_id();
    let mut processes = PROCESS_TABLE.lock();
    let process = processes
        .iter_mut()
        .flatten()
        .find(|process| process.task_id == task_id)
        .ok_or(FileError::NoProcess)?;
    let descriptor = (0..MAX_FILE_DESCRIPTORS)
        .find(|descriptor| process.files[*descriptor].is_none())
        .ok_or(FileError::TooManyFiles)?;
    process.files[descriptor] = Some(FdKind::File(file));
    Ok(descriptor as u64)
}

pub fn read_current(descriptor: u64, buffer: &mut [u8]) -> Result<usize, FileError> {
    let descriptor = usize::try_from(descriptor).map_err(|_| FileError::BadDescriptor)?;
    let task_id = current_task_id();
    let processes = PROCESS_TABLE.lock();
    let process = processes
        .iter()
        .flatten()
        .find(|process| process.task_id == task_id)
        .ok_or(FileError::NoProcess)?;
    let fd = process
        .files
        .get(descriptor)
        .copied()
        .flatten()
        .ok_or(FileError::BadDescriptor)?;
    drop(processes);
    match fd {
        FdKind::File(id) => vfs::read(id, buffer).map_err(|_| FileError::BadDescriptor),
        FdKind::PipeRead(id) => crate::pipe::read(id, buffer).map_err(|_| FileError::BadDescriptor),
        FdKind::PipeWrite(_) => Err(FileError::BadDescriptor),
    }
}

pub fn write_current(descriptor: u64, buffer: &[u8]) -> Result<usize, FileError> {
    let descriptor = usize::try_from(descriptor).map_err(|_| FileError::BadDescriptor)?;
    let task_id = current_task_id();
    let processes = PROCESS_TABLE.lock();
    let process = processes
        .iter()
        .flatten()
        .find(|process| process.task_id == task_id)
        .ok_or(FileError::NoProcess)?;
    let fd = process
        .files
        .get(descriptor)
        .copied()
        .flatten()
        .ok_or(FileError::BadDescriptor)?;
    drop(processes);
    match fd {
        FdKind::File(id) => {
            if !current_has(Capability::FileWrite) {
                return Err(FileError::PermissionDenied);
            }
            vfs::write(id, buffer).map_err(|_| FileError::BadDescriptor)
        }
        FdKind::PipeWrite(id) => crate::pipe::write(id, buffer).map_err(|_| FileError::BadDescriptor),
        FdKind::PipeRead(_) => Err(FileError::BadDescriptor),
    }
}

pub fn dup_current(descriptor: u64) -> Result<u64, FileError> {
    let descriptor = usize::try_from(descriptor).map_err(|_| FileError::BadDescriptor)?;
    let task_id = current_task_id();
    let mut processes = PROCESS_TABLE.lock();
    let process = processes
        .iter_mut()
        .flatten()
        .find(|process| process.task_id == task_id)
        .ok_or(FileError::NoProcess)?;
    let fd = process
        .files
        .get(descriptor)
        .copied()
        .flatten()
        .ok_or(FileError::BadDescriptor)?;
    let cloned = match fd {
        FdKind::File(id) => {
            FdKind::File(vfs::clone_open_file(id).map_err(|_| FileError::TooManyFiles)?)
        }
        FdKind::PipeRead(id) => {
            crate::pipe::clone_reader(id).map_err(|_| FileError::TooManyFiles)?;
            FdKind::PipeRead(id)
        }
        FdKind::PipeWrite(id) => {
            crate::pipe::clone_writer(id).map_err(|_| FileError::TooManyFiles)?;
            FdKind::PipeWrite(id)
        }
    };
    let new_descriptor = (0..MAX_FILE_DESCRIPTORS)
        .find(|slot| process.files[*slot].is_none())
        .ok_or(FileError::TooManyFiles)?;
    process.files[new_descriptor] = Some(cloned);
    Ok(new_descriptor as u64)
}

pub fn dup2_current(old: u64, new: u64) -> Result<u64, FileError> {
    let old = usize::try_from(old).map_err(|_| FileError::BadDescriptor)?;
    let new = usize::try_from(new).map_err(|_| FileError::BadDescriptor)?;
    if new >= MAX_FILE_DESCRIPTORS {
        return Err(FileError::BadDescriptor);
    }
    if old == new {
        return Ok(new as u64);
    }
    let task_id = current_task_id();
    let mut processes = PROCESS_TABLE.lock();
    let process = processes
        .iter_mut()
        .flatten()
        .find(|process| process.task_id == task_id)
        .ok_or(FileError::NoProcess)?;
    let fd = process
        .files
        .get(old)
        .copied()
        .flatten()
        .ok_or(FileError::BadDescriptor)?;
    let cloned = match fd {
        FdKind::File(id) => {
            FdKind::File(vfs::clone_open_file(id).map_err(|_| FileError::TooManyFiles)?)
        }
        FdKind::PipeRead(id) => {
            crate::pipe::clone_reader(id).map_err(|_| FileError::TooManyFiles)?;
            FdKind::PipeRead(id)
        }
        FdKind::PipeWrite(id) => {
            crate::pipe::clone_writer(id).map_err(|_| FileError::TooManyFiles)?;
            FdKind::PipeWrite(id)
        }
    };
    if let Some(prev) = process.files[new].take() {
        release_fd(prev);
    }
    process.files[new] = Some(cloned);
    Ok(new as u64)
}

pub fn pipe_current() -> Result<(u64, u64), FileError> {
    let pipe_id = crate::pipe::create().map_err(|_| FileError::TooManyFiles)?;
    let task_id = current_task_id();
    let mut processes = PROCESS_TABLE.lock();
    let process = processes
        .iter_mut()
        .flatten()
        .find(|process| process.task_id == task_id)
        .ok_or(FileError::NoProcess)?;
    let read_fd = (0..MAX_FILE_DESCRIPTORS)
        .find(|slot| process.files[*slot].is_none())
        .ok_or(FileError::TooManyFiles)?;
    process.files[read_fd] = Some(FdKind::PipeRead(pipe_id));
    let write_fd = (0..MAX_FILE_DESCRIPTORS)
        .find(|slot| process.files[*slot].is_none())
        .ok_or(FileError::TooManyFiles)?;
    process.files[write_fd] = Some(FdKind::PipeWrite(pipe_id));
    Ok((read_fd as u64, write_fd as u64))
}

pub fn getppid_current() -> u64 {
    let processes = PROCESS_TABLE.lock();
    let task_id = current_task_id();
    processes
        .iter()
        .flatten()
        .find(|p| p.task_id == task_id)
        .map(|p| p.parent.as_u64())
        .unwrap_or(0)
}

pub fn close_current(descriptor: u64) -> Result<(), FileError> {
    let descriptor = usize::try_from(descriptor).map_err(|_| FileError::BadDescriptor)?;
    // Standard streams cannot be closed.
    if descriptor < 3 {
        return Err(FileError::BadDescriptor);
    }
    let task_id = current_task_id();
    let mut processes = PROCESS_TABLE.lock();
    let process = processes
        .iter_mut()
        .flatten()
        .find(|process| process.task_id == task_id)
        .ok_or(FileError::NoProcess)?;
    let fd = process
        .files
        .get_mut(descriptor)
        .ok_or(FileError::BadDescriptor)?
        .take()
        .ok_or(FileError::BadDescriptor)?;
    drop(processes);
    release_fd(fd);
    Ok(())
}

pub fn open_file_count() -> usize {
    PROCESS_TABLE
        .lock()
        .iter()
        .flatten()
        .map(|process| process.files.iter().flatten().count())
        .sum()
}

pub fn stat_path(path: &str) -> Result<vfs::Stat, FileError> {
    if !current_has(Capability::FileRead) {
        return Err(FileError::PermissionDenied);
    }
    let path = resolve_path_for_current(path)?;
    vfs::stat(&path).map_err(|_| FileError::NotFound)
}

pub fn readdir_path(path: &str, index: usize) -> Result<vfs::DirEntry, FileError> {
    if !current_has(Capability::FileRead) {
        return Err(FileError::PermissionDenied);
    }
    let path = resolve_path_for_current(path)?;
    vfs::readdir(&path, index).map_err(|_| FileError::NotFound)
}

pub fn mkdir_path(path: &str) -> Result<(), FileError> {
    if !current_has(Capability::FileWrite) {
        return Err(FileError::PermissionDenied);
    }
    let path = resolve_path_for_current(path)?;
    match vfs::mkdir(&path) {
        Ok(()) => Ok(()),
        Err(vfs::Error::AlreadyExists) => Err(FileError::AlreadyExists),
        Err(vfs::Error::Full) => Err(FileError::TooManyFiles),
        Err(_) => Err(FileError::NotFound),
    }
}


pub fn current_cwd_str(buf: &mut [u8]) -> Result<usize, FileError> {
    let (cwd, len) = current_cwd();
    if buf.len() < len {
        return Err(FileError::TooManyFiles);
    }
    buf[..len].copy_from_slice(&cwd[..len]);
    Ok(len)
}

pub fn current_cwd() -> ([u8; crate::config::MAX_PATH_SIZE], usize) {
    let processes = PROCESS_TABLE.lock();
    let task_id = current_task_id();
    if let Some(p) = processes.iter().flatten().find(|p| p.task_id == task_id) {
        return (p.cwd, p.cwd_len);
    }
    let mut root = [0u8; crate::config::MAX_PATH_SIZE];
    root[0] = b'/';
    (root, 1)
}

pub fn chdir_current(path: &str) -> Result<(), FileError> {
    if !current_has(Capability::FileRead) {
        return Err(FileError::PermissionDenied);
    }
    let absolute = resolve_path_for_current(path)?;
    match vfs::stat(&absolute) {
        Ok(stat) if stat.kind == vfs::NodeKind::Directory => {}
        _ => return Err(FileError::NotFound),
    }
    let bytes = absolute.as_bytes();
    if bytes.len() > crate::config::MAX_PATH_SIZE {
        return Err(FileError::NotFound);
    }
    let mut processes = PROCESS_TABLE.lock();
    let task_id = current_task_id();
    let process = processes
        .iter_mut()
        .flatten()
        .find(|p| p.task_id == task_id)
        .ok_or(FileError::NoProcess)?;
    process.cwd = [0; crate::config::MAX_PATH_SIZE];
    process.cwd[..bytes.len()].copy_from_slice(bytes);
    process.cwd_len = bytes.len();
    Ok(())
}

pub fn resolve_path_for_current(path: &str) -> Result<alloc::string::String, FileError> {
    let cwd = current_cwd();
    resolve_path_with_cwd(path, &cwd)
}

fn resolve_path_with_cwd(
    path: &str,
    cwd: &([u8; crate::config::MAX_PATH_SIZE], usize),
) -> Result<alloc::string::String, FileError> {
    if path.is_empty() {
        return Err(FileError::NotFound);
    }
    if path.starts_with('/') {
        return normalize_absolute(path);
    }
    let cwd_str = core::str::from_utf8(&cwd.0[..cwd.1]).map_err(|_| FileError::NotFound)?;
    let mut joined = alloc::string::String::new();
    if cwd_str == "/" {
        joined.push('/');
        joined.push_str(path);
    } else {
        joined.push_str(cwd_str);
        joined.push('/');
        joined.push_str(path);
    }
    normalize_absolute(&joined)
}

fn normalize_absolute(path: &str) -> Result<alloc::string::String, FileError> {
    if !path.starts_with('/') {
        return Err(FileError::NotFound);
    }
    let mut stack = alloc::vec::Vec::new();
    for component in path.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            let _ = stack.pop();
            continue;
        }
        if component.as_bytes().contains(&0) {
            return Err(FileError::NotFound);
        }
        stack.push(component);
    }
    if stack.is_empty() {
        return Ok(alloc::string::String::from("/"));
    }
    let mut out = alloc::string::String::from("/");
    for (i, part) in stack.iter().enumerate() {
        if i > 0 {
            out.push('/');
        }
        out.push_str(part);
    }
    if out.len() > crate::config::MAX_PATH_SIZE {
        return Err(FileError::NotFound);
    }
    Ok(out)
}

pub fn process_count() -> usize {
    PROCESS_TABLE
        .lock()
        .iter()
        .flatten()
        .filter(|process| process.state != ProcessState::Exited)
        .count()
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

fn enter_user_context(context: UserTaskContext) -> ! {
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
    let interrupts_were_enabled = x86_64::instructions::interrupts::are_enabled();
    x86_64::instructions::interrupts::disable();

    let switch = {
        let mut scheduler = SCHEDULER.lock();
        assert!(scheduler.task_count != 0, "scheduler not initialized");
        scheduler.prepare_switch()
    };

    if let Some(context_switch) = switch {
        // SAFETY: Interrupts stay disabled between publishing scheduler state
        // and switching stacks, so an IRQ cannot save a context into the wrong
        // task control block.
        unsafe { switch_stacks(context_switch) };
    }

    if interrupts_were_enabled {
        x86_64::instructions::interrupts::enable();
    }
}

pub fn tick() {
    let Some(mut scheduler) = SCHEDULER.try_lock() else {
        PREEMPTION_REQUESTED.store(true, Ordering::Release);
        return;
    };
    scheduler.wake_sleeping(timer::ticks());
    let slot = scheduler.current_slot;
    let task = &mut scheduler.tasks[slot];
    if task.state != TaskState::Running {
        return;
    }
    if task.remaining_ticks > 1 {
        task.remaining_ticks -= 1;
    } else {
        task.remaining_ticks = 0;
        PREEMPTION_REQUESTED.store(true, Ordering::Release);
    }
}
pub fn preempt_from_interrupt() {
    if !PREEMPTION_REQUESTED.swap(false, Ordering::AcqRel) {
        return;
    }

    let switch = {
        let Some(mut scheduler) = SCHEDULER.try_lock() else {
            PREEMPTION_REQUESTED.store(true, Ordering::Release);
            return;
        };
        scheduler.wake_sleeping(timer::ticks());
        scheduler.prepare_switch()
    };

    if let Some(context_switch) = switch {
        PREEMPTION_SWITCHES.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Hardware interrupts are disabled by the interrupt gate. The
        // scheduler lock is released, and both contexts belong to live tasks.
        unsafe { switch_stacks(context_switch) };
    }
}

pub fn preemption_point() {
    if PREEMPTION_REQUESTED.load(Ordering::Acquire) {
        yield_now();
        PREEMPTION_REQUESTED.store(false, Ordering::Release);
    }
}

pub fn block_current() {
    let interrupts_were_enabled = x86_64::instructions::interrupts::are_enabled();
    x86_64::instructions::interrupts::disable();

    let switch = {
        let mut scheduler = SCHEDULER.lock();
        let slot = scheduler.current_slot;
        assert!(
            scheduler.tasks[slot].id != KERNEL_TASK_ID,
            "kernel task cannot block"
        );
        scheduler.tasks[slot].state = TaskState::Blocked;
        scheduler.prepare_switch()
    };

    if let Some(context_switch) = switch {
        // SAFETY: The blocked task remains allocated and can only run again
        // after an explicit wakeup changes its state back to ready.
        unsafe { switch_stacks(context_switch) };
    }

    if interrupts_were_enabled {
        x86_64::instructions::interrupts::enable();
    }
}

pub fn wake_task(id: TaskId) -> bool {
    let mut scheduler = SCHEDULER.lock();
    let Some(task) = scheduler
        .tasks
        .iter_mut()
        .find(|task| task.id == id && task.state == TaskState::Blocked)
    else {
        return false;
    };

    task.state = TaskState::Ready;
    true
}

pub fn sleep_current(ticks: u64) {
    let interrupts_were_enabled = x86_64::instructions::interrupts::are_enabled();
    x86_64::instructions::interrupts::disable();

    let switch = {
        let mut scheduler = SCHEDULER.lock();
        let slot = scheduler.current_slot;
        assert!(
            scheduler.tasks[slot].id != KERNEL_TASK_ID,
            "kernel task cannot sleep"
        );
        scheduler.tasks[slot].wake_tick = timer::ticks().saturating_add(ticks.max(1));
        scheduler.tasks[slot].state = TaskState::Sleeping;
        scheduler.prepare_switch()
    };

    if let Some(context_switch) = switch {
        // SAFETY: The sleeping task remains allocated and will only become
        // ready after its wake deadline is observed by the timer path.
        unsafe { switch_stacks(context_switch) };
    }

    if interrupts_were_enabled {
        x86_64::instructions::interrupts::enable();
    }
}

pub fn exit_current_task() -> ! {
    x86_64::instructions::interrupts::disable();
    let switch = {
        let mut scheduler = SCHEDULER.lock();
        let slot = scheduler.current_slot;
        assert!(
            scheduler.tasks[slot].id != KERNEL_TASK_ID,
            "kernel task cannot exit"
        );
        scheduler.tasks[slot].state = TaskState::Dead;
        scheduler.task_count = scheduler.task_count.saturating_sub(1);
        scheduler.prepare_switch()
    };

    if let Some(context_switch) = switch {
        // SAFETY: The exiting task will never be selected again, and the next
        // context belongs to a live task selected by the scheduler.
        unsafe { switch_stacks(context_switch) };
    }

    loop {
        x86_64::instructions::hlt();
    }
}

pub fn summary() -> Summary {
    let scheduler = SCHEDULER.lock();
    assert!(scheduler.task_count != 0, "scheduler not initialized");
    scheduler.summary()
}

pub fn current_task_id() -> TaskId {
    let scheduler = SCHEDULER.lock();
    assert!(scheduler.task_count != 0, "scheduler not initialized");
    scheduler.tasks[scheduler.current_slot].id
}

pub fn current_has(capability: Capability) -> bool {
    let scheduler = SCHEDULER.lock();
    assert!(scheduler.task_count != 0, "scheduler not initialized");
    scheduler.tasks[scheduler.current_slot]
        .capabilities
        .contains(capability)
}

pub fn grant(target: TaskId, capability: Capability) -> Result<(), CapabilityError> {
    let actor = current_process_id();
    let result = {
        let mut scheduler = SCHEDULER.lock();
        assert!(scheduler.task_count != 0, "scheduler not initialized");

        let authority = scheduler.tasks[scheduler.current_slot].capabilities;
        if !authority.contains(Capability::TaskControl) || !authority.contains(capability) {
            Err(CapabilityError::PermissionDenied)
        } else if let Some(target_task) = scheduler
            .tasks
            .iter_mut()
            .find(|task| task.state != TaskState::Empty && task.id == target)
        {
            target_task.capabilities = target_task.capabilities.with(capability);
            Ok(())
        } else {
            Err(CapabilityError::UnknownTask)
        }
    };
    crate::audit::record(
        actor,
        crate::audit::Action::CapabilityGrant,
        target.as_u64(),
        result.is_ok(),
    );
    result
}

pub fn revoke(target: TaskId, capability: Capability) -> Result<(), CapabilityError> {
    let actor = current_process_id();
    let result = {
        let mut scheduler = SCHEDULER.lock();
        assert!(scheduler.task_count != 0, "scheduler not initialized");

        if !scheduler.tasks[scheduler.current_slot]
            .capabilities
            .contains(Capability::TaskControl)
        {
            Err(CapabilityError::PermissionDenied)
        } else if let Some(target_task) = scheduler
            .tasks
            .iter_mut()
            .find(|task| task.state != TaskState::Empty && task.id == target)
        {
            target_task.capabilities = target_task.capabilities.without(capability);
            Ok(())
        } else {
            Err(CapabilityError::UnknownTask)
        }
    };
    crate::audit::record(
        actor,
        crate::audit::Action::CapabilityRevoke,
        target.as_u64(),
        result.is_ok(),
    );
    result
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


pub fn kill_process(pid: u64, sig: u64) -> Result<(), FileError> {
    // Minimal POSIX-ish: SIGTERM(15) / SIGKILL(9) mark process for exit.
    if sig != 9 && sig != 15 && sig != 0 {
        return Err(FileError::NotFound);
    }
    let mut processes = PROCESS_TABLE.lock();
    let process = processes
        .iter_mut()
        .flatten()
        .find(|p| p.id.as_u64() == pid)
        .ok_or(FileError::NotFound)?;
    if sig == 0 {
        return Ok(()); // existence check
    }
    process.pending_signal = sig;
    if process.state != ProcessState::Exited {
        process.state = ProcessState::Exited;
        process.exit_code = 128 + (sig as i32);
        // Unblock pipe peers and drop resources.
        release_file_table(&mut process.files);
    }
    Ok(())
}


pub fn seek_current(descriptor: u64, offset: u64) -> Result<u64, FileError> {
    let descriptor = usize::try_from(descriptor).map_err(|_| FileError::BadDescriptor)?;
    let offset = usize::try_from(offset).map_err(|_| FileError::BadDescriptor)?;
    let task_id = current_task_id();
    let processes = PROCESS_TABLE.lock();
    let process = processes
        .iter()
        .flatten()
        .find(|process| process.task_id == task_id)
        .ok_or(FileError::NoProcess)?;
    let fd = process
        .files
        .get(descriptor)
        .copied()
        .flatten()
        .ok_or(FileError::BadDescriptor)?;
    drop(processes);
    match fd {
        FdKind::File(id) => vfs::seek(id, offset)
            .map(|pos| pos as u64)
            .map_err(|_| FileError::BadDescriptor),
        _ => Err(FileError::BadDescriptor),
    }
}

pub fn unlink_current(path: &str) -> Result<(), FileError> {
    if !current_has(Capability::FileWrite) {
        return Err(FileError::PermissionDenied);
    }
    vfs::remove(path).map_err(|e| match e {
        vfs::Error::NotFound => FileError::NotFound,
        vfs::Error::ReadOnly => FileError::PermissionDenied,
        _ => FileError::NotFound,
    })
}


pub fn rename_current(old: &str, new: &str) -> Result<(), FileError> {
    if !current_has(Capability::FileWrite) {
        return Err(FileError::PermissionDenied);
    }
    vfs::rename(old, new).map_err(|e| match e {
        vfs::Error::NotFound => FileError::NotFound,
        vfs::Error::AlreadyExists => FileError::AlreadyExists,
        vfs::Error::ReadOnly => FileError::PermissionDenied,
        _ => FileError::NotFound,
    })
}
