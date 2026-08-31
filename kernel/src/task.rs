use spin::{Mutex, Once};

const MAX_TASKS: usize = 16;
const KERNEL_TASK_ID: TaskId = TaskId(0);

static SCHEDULER: Once<Mutex<Scheduler>> = Once::new();

#[derive(Clone, Copy)]
pub struct TaskId(u64);

impl TaskId {
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy)]
enum TaskSlot {
    Empty,
    Running(TaskControlBlock),
}

#[derive(Clone, Copy)]
struct TaskControlBlock {
    id: TaskId,
    name: &'static str,
}

pub struct Summary {
    pub task_count: usize,
    pub current_id: TaskId,
    pub current_name: &'static str,
}

struct Scheduler {
    tasks: [TaskSlot; MAX_TASKS],
    current_slot: usize,
    task_count: usize,
}

impl Scheduler {
    const fn new() -> Self {
        let mut tasks = [TaskSlot::Empty; MAX_TASKS];
        tasks[0] = TaskSlot::Running(TaskControlBlock {
            id: KERNEL_TASK_ID,
            name: "kernel",
        });

        Self {
            tasks,
            current_slot: 0,
            task_count: 1,
        }
    }

    fn summary(&self) -> Summary {
        let TaskSlot::Running(task) = self.tasks[self.current_slot] else {
            panic!("scheduler current slot is not runnable");
        };

        Summary {
            task_count: self.task_count,
            current_id: task.id,
            current_name: task.name,
        }
    }
}

pub fn init() {
    SCHEDULER.call_once(|| Mutex::new(Scheduler::new()));
}

pub fn summary() -> Summary {
    SCHEDULER
        .get()
        .expect("scheduler must be initialized before use")
        .lock()
        .summary()
}
