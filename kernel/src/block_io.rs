use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use spin::Mutex;

use crate::{
    block::{BlockDevice, Error, SECTOR_SIZE},
    config::MAX_BLOCK_IO_REQUESTS,
    task::{self, TaskId, TaskPriority},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Read,
    Write,
    Flush,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RequestState {
    Pending,
    InProgress,
    Complete,
}

#[derive(Clone, Copy)]
struct Request {
    id: u64,
    operation: Operation,
    lba: u64,
    data: [u8; SECTOR_SIZE],
    result: Result<(), Error>,
    state: RequestState,
}

impl Request {
    fn pending(id: u64, operation: Operation, lba: u64, data: [u8; SECTOR_SIZE]) -> Self {
        Self {
            id,
            operation,
            lba,
            data,
            result: Ok(()),
            state: RequestState::Pending,
        }
    }
}

#[derive(Clone, Copy)]
struct WorkItem {
    slot: usize,
    request: Request,
}

#[derive(Clone, Copy)]
struct Queue {
    entries: [Option<Request>; MAX_BLOCK_IO_REQUESTS],
    next_id: u64,
}

impl Queue {
    const fn new() -> Self {
        Self {
            entries: [const { None }; MAX_BLOCK_IO_REQUESTS],
            next_id: 1,
        }
    }

    fn push(&mut self, operation: Operation, lba: u64, data: [u8; SECTOR_SIZE]) -> Option<u64> {
        let slot = self.entries.iter_mut().find(|entry| entry.is_none())?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        *slot = Some(Request::pending(id, operation, lba, data));
        Some(id)
    }

    fn take_pending(&mut self) -> Option<WorkItem> {
        for (slot, entry) in self.entries.iter_mut().enumerate() {
            let Some(request) = entry.as_mut() else {
                continue;
            };
            if request.state == RequestState::Pending {
                request.state = RequestState::InProgress;
                return Some(WorkItem {
                    slot,
                    request: *request,
                });
            }
        }
        None
    }

    fn finish(&mut self, slot: usize, id: u64, result: Result<(), Error>, data: [u8; SECTOR_SIZE]) {
        let Some(Some(request)) = self.entries.get_mut(slot) else {
            return;
        };
        if request.id == id && request.state == RequestState::InProgress {
            request.data = data;
            request.result = result;
            request.state = RequestState::Complete;
        }
    }

    fn take_result(&mut self, id: u64, output: Option<&mut [u8]>) -> Option<Result<(), Error>> {
        for entry in &mut self.entries {
            let Some(request) = entry else {
                continue;
            };
            if request.id != id || request.state != RequestState::Complete {
                continue;
            }
            let result = request.result;
            if result.is_ok() {
                if let Some(output) = output {
                    output.copy_from_slice(&request.data);
                }
            }
            *entry = None;
            return Some(result);
        }
        None
    }

    fn pending_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.is_some_and(|request| request.state == RequestState::Pending))
            .count()
    }

    fn active_count(&self) -> usize {
        self.entries.iter().filter(|entry| entry.is_some()).count()
    }
}

static QUEUE: Mutex<Queue> = Mutex::new(Queue::new());
static WORKER_TASK: Mutex<Option<TaskId>> = Mutex::new(None);
static IN_WORKER: AtomicBool = AtomicBool::new(false);
static QUEUED: AtomicU64 = AtomicU64::new(0);
static COMPLETED: AtomicU64 = AtomicU64::new(0);
static DIRECT: AtomicU64 = AtomicU64::new(0);
static PROBE_DONE: AtomicBool = AtomicBool::new(false);
static PROBE_PASSED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub queued: u64,
    pub completed: u64,
    pub direct: u64,
    pub pending: usize,
    pub active: usize,
}

pub struct PrimaryAta;

pub fn primary_ata() -> PrimaryAta {
    PrimaryAta
}

pub fn primary_ata_present() -> bool {
    crate::ata::with_primary_master(|_| ()).is_some()
}

pub fn start_worker() -> bool {
    if WORKER_TASK.lock().is_some() {
        return true;
    }
    match task::spawn_with_priority("block-io", worker_task, TaskPriority::NORMAL) {
        Ok(id) => {
            *WORKER_TASK.lock() = Some(id);
            true
        }
        Err(_) => false,
    }
}

pub fn async_completion_self_test() -> bool {
    if WORKER_TASK.lock().is_none() {
        return false;
    }
    PROBE_DONE.store(false, Ordering::Release);
    PROBE_PASSED.store(false, Ordering::Release);
    if task::spawn("block-io-probe", async_probe_task).is_err() {
        return false;
    }
    let start = crate::timer::ticks();
    while !PROBE_DONE.load(Ordering::Acquire) {
        if crate::timer::ticks().wrapping_sub(start) > 100 {
            return false;
        }
        task::yield_now();
    }
    PROBE_PASSED.load(Ordering::Acquire)
}

fn async_probe_task() -> ! {
    let before = stats();
    let mut sector = [0_u8; SECTOR_SIZE];
    let result = primary_ata().read_sector(0, &mut sector);
    let after = stats();
    let completed_result = matches!(
        result,
        Ok(()) | Err(Error::DeviceFault) | Err(Error::OutOfBounds)
    );
    PROBE_PASSED.store(
        completed_result
            && after.queued > before.queued
            && after.completed > before.completed
            && after.direct == before.direct,
        Ordering::Release,
    );
    PROBE_DONE.store(true, Ordering::Release);
    task::exit_current_task()
}

pub fn stats() -> Stats {
    let queue = QUEUE.lock();
    Stats {
        queued: QUEUED.load(Ordering::Acquire),
        completed: COMPLETED.load(Ordering::Acquire),
        direct: DIRECT.load(Ordering::Acquire),
        pending: queue.pending_count(),
        active: queue.active_count(),
    }
}

impl BlockDevice for PrimaryAta {
    fn sector_count(&self) -> u64 {
        crate::ata::with_primary_master(|disk| disk.sector_count()).unwrap_or(0)
    }

    fn is_read_only(&self) -> bool {
        crate::ata::with_primary_master(|disk| disk.is_read_only()).unwrap_or(true)
    }

    fn flush(&mut self) -> Result<(), Error> {
        submit(Operation::Flush, 0, None, None)
    }

    fn read_sector(&mut self, lba: u64, sector: &mut [u8]) -> Result<(), Error> {
        if sector.len() != SECTOR_SIZE {
            return Err(Error::InvalidBuffer);
        }
        submit(Operation::Read, lba, None, Some(sector))
    }

    fn write_sector(&mut self, lba: u64, sector: &[u8]) -> Result<(), Error> {
        if sector.len() != SECTOR_SIZE {
            return Err(Error::InvalidBuffer);
        }
        submit(Operation::Write, lba, Some(sector), None)
    }
}

fn should_queue() -> bool {
    if IN_WORKER.load(Ordering::Acquire) || !x86_64::instructions::interrupts::are_enabled() {
        return false;
    }
    let Some(current) = task::current_task_id_if_running() else {
        return false;
    };
    if current.as_u64() == 0 {
        return false;
    }
    WORKER_TASK.lock().is_some_and(|worker| worker != current)
}

fn submit(
    operation: Operation,
    lba: u64,
    input: Option<&[u8]>,
    output: Option<&mut [u8]>,
) -> Result<(), Error> {
    if !should_queue() {
        DIRECT.fetch_add(1, Ordering::Relaxed);
        return direct_primary(operation, lba, input, output);
    }

    let mut data = [0_u8; SECTOR_SIZE];
    if let Some(input) = input {
        data.copy_from_slice(input);
    }
    let Some(id) = QUEUE.lock().push(operation, lba, data) else {
        return Err(Error::DeviceFault);
    };
    QUEUED.fetch_add(1, Ordering::Relaxed);
    if let Some(worker) = *WORKER_TASK.lock() {
        let _ = task::wake_task(worker);
    }
    wait_for_completion(id, output)
}

fn wait_for_completion(mut id: u64, mut output: Option<&mut [u8]>) -> Result<(), Error> {
    loop {
        if let Some(result) = QUEUE.lock().take_result(id, output.as_deref_mut()) {
            return result;
        }
        task::yield_now();
        id = id.max(1);
    }
}

fn worker_task() -> ! {
    loop {
        if !process_one_primary() {
            task::sleep_current(16);
        }
    }
}

fn process_one_primary() -> bool {
    let Some(work) = QUEUE.lock().take_pending() else {
        return false;
    };
    IN_WORKER.store(true, Ordering::Release);
    let mut data = work.request.data;
    let result = match work.request.operation {
        Operation::Read => direct_primary(Operation::Read, work.request.lba, None, Some(&mut data)),
        Operation::Write => direct_primary(Operation::Write, work.request.lba, Some(&data), None),
        Operation::Flush => direct_primary(Operation::Flush, work.request.lba, None, None),
    };
    IN_WORKER.store(false, Ordering::Release);
    QUEUE
        .lock()
        .finish(work.slot, work.request.id, result, data);
    COMPLETED.fetch_add(1, Ordering::Relaxed);
    true
}

fn direct_primary(
    operation: Operation,
    lba: u64,
    input: Option<&[u8]>,
    output: Option<&mut [u8]>,
) -> Result<(), Error> {
    crate::ata::with_primary_master(|disk| match operation {
        Operation::Read => {
            let Some(output) = output else {
                return Err(Error::InvalidBuffer);
            };
            disk.read_sector(lba, output)
        }
        Operation::Write => {
            let Some(input) = input else {
                return Err(Error::InvalidBuffer);
            };
            disk.write_sector(lba, input)
        }
        Operation::Flush => disk.flush(),
    })
    .unwrap_or(Err(Error::DeviceFault))
}

fn drive_one(queue: &mut Queue, device: &mut impl BlockDevice) -> bool {
    let Some(work) = queue.take_pending() else {
        return false;
    };
    let mut data = work.request.data;
    let result = match work.request.operation {
        Operation::Read => device.read_sector(work.request.lba, &mut data),
        Operation::Write => device.write_sector(work.request.lba, &data),
        Operation::Flush => device.flush(),
    };
    queue.finish(work.slot, work.request.id, result, data);
    true
}

pub fn self_test() -> bool {
    let mut queue = Queue::new();
    let mut disk = crate::block::RamDisk::<4>::new();
    let mut first = [0_u8; SECTOR_SIZE];
    let mut second = [0_u8; SECTOR_SIZE];
    first[..12].copy_from_slice(b"wovenhat-io!");
    second.fill(0xaa);

    let Some(write_id) = queue.push(Operation::Write, 2, first) else {
        return false;
    };
    if queue.pending_count() != 1 || !drive_one(&mut queue, &mut disk) {
        return false;
    }
    let mut out = [0_u8; SECTOR_SIZE];
    if queue.take_result(write_id, None) != Some(Ok(()))
        || disk.read_sector(2, &mut out).is_err()
        || out != first
    {
        return false;
    }

    let Some(read_id) = queue.push(Operation::Read, 2, [0; SECTOR_SIZE]) else {
        return false;
    };
    if !drive_one(&mut queue, &mut disk)
        || queue.take_result(read_id, Some(&mut out)) != Some(Ok(()))
        || out != first
    {
        return false;
    }

    let mut ids = [0_u64; MAX_BLOCK_IO_REQUESTS];
    for (index, slot) in ids.iter_mut().enumerate() {
        let Some(id) = queue.push(Operation::Write, index as u64 % 4, second) else {
            return false;
        };
        *slot = id;
    }
    if queue.push(Operation::Flush, 0, [0; SECTOR_SIZE]).is_some()
        || queue.active_count() != MAX_BLOCK_IO_REQUESTS
    {
        return false;
    }
    while drive_one(&mut queue, &mut disk) {}
    for id in ids {
        if queue.take_result(id, None) != Some(Ok(())) {
            return false;
        }
    }
    if queue.active_count() != 0 {
        return false;
    }

    disk.set_read_only(true);
    let Some(fail_id) = queue.push(Operation::Write, 1, first) else {
        return false;
    };
    drive_one(&mut queue, &mut disk)
        && queue.take_result(fail_id, None) == Some(Err(Error::ReadOnly))
}
