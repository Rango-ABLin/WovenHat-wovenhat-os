use spin::Mutex;

const CAPACITY: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    CapabilityGrant,
    CapabilityRevoke,
    IpcSend,
    FileWrite,
}

#[derive(Clone, Copy)]
pub struct Event {
    pub sequence: u64,
    pub tick: u64,
    pub actor: u64,
    pub action: Action,
    pub target: u64,
    pub allowed: bool,
}

struct Log {
    events: [Option<Event>; CAPACITY],
    next: usize,
    count: usize,
    sequence: u64,
}

impl Log {
    const fn new() -> Self {
        Self {
            events: [None; CAPACITY],
            next: 0,
            count: 0,
            sequence: 0,
        }
    }

    fn record(&mut self, mut event: Event) {
        event.sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        self.events[self.next] = Some(event);
        self.next = (self.next + 1) % CAPACITY;
        self.count = core::cmp::min(self.count + 1, CAPACITY);
    }

    fn latest(&self) -> Option<Event> {
        if self.count == 0 {
            None
        } else {
            self.events[(self.next + CAPACITY - 1) % CAPACITY]
        }
    }
}

static LOG: Mutex<Log> = Mutex::new(Log::new());

pub fn record(actor: u64, action: Action, target: u64, allowed: bool) {
    LOG.lock().record(Event {
        sequence: 0,
        tick: crate::timer::ticks(),
        actor,
        action,
        target,
        allowed,
    });
}

pub fn latest() -> Option<Event> {
    LOG.lock().latest()
}

pub fn count() -> usize {
    LOG.lock().count
}

pub fn self_test() -> bool {
    let mut log = Log::new();
    for index in 0..=CAPACITY {
        log.record(Event {
            sequence: 0,
            tick: index as u64,
            actor: 1,
            action: Action::CapabilityGrant,
            target: 2,
            allowed: index % 2 == 0,
        });
    }
    log.count == CAPACITY
        && log.latest().is_some_and(|event| {
            event.sequence == CAPACITY as u64
                && event.tick == CAPACITY as u64
                && event.actor == 1
                && event.target == 2
                && event.allowed
        })
}
