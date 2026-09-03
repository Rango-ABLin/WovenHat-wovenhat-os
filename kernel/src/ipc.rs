use spin::Mutex;

use crate::config::{
    IPC_QUEUE_DEPTH as QUEUE_DEPTH, MAX_IPC_ENDPOINTS as MAX_ENDPOINTS,
};

pub use crate::config::MAX_MESSAGE_SIZE;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Message {
    pub sender: u64,
    length: usize,
    bytes: [u8; MAX_MESSAGE_SIZE],
}

impl Message {
    const EMPTY: Self = Self {
        sender: 0,
        length: 0,
        bytes: [0; MAX_MESSAGE_SIZE],
    };

    pub fn payload(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Error {
    EndpointExists,
    NoEndpoint,
    RegistryFull,
    QueueFull,
    QueueEmpty,
    MessageTooLarge,
}

#[derive(Clone, Copy)]
struct Endpoint {
    owner: u64,
    messages: [Message; QUEUE_DEPTH],
    length: usize,
}

impl Endpoint {
    const EMPTY: Self = Self {
        owner: 0,
        messages: [Message::EMPTY; QUEUE_DEPTH],
        length: 0,
    };

    fn enqueue(&mut self, message: Message) -> Result<(), Error> {
        if self.length == QUEUE_DEPTH {
            return Err(Error::QueueFull);
        }
        self.messages[self.length] = message;
        self.length += 1;
        Ok(())
    }

    fn dequeue(&mut self) -> Result<Message, Error> {
        if self.length == 0 {
            return Err(Error::QueueEmpty);
        }
        let message = self.messages[0];
        self.messages.copy_within(1..self.length, 0);
        self.length -= 1;
        self.messages[self.length] = Message::EMPTY;
        Ok(message)
    }
}

struct State {
    endpoints: [Option<Endpoint>; MAX_ENDPOINTS],
}

impl State {
    const fn new() -> Self {
        Self {
            endpoints: [None; MAX_ENDPOINTS],
        }
    }

    fn register(&mut self, owner: u64) -> Result<(), Error> {
        if self.endpoint(owner).is_some() {
            return Err(Error::EndpointExists);
        }
        let slot = self
            .endpoints
            .iter_mut()
            .find(|endpoint| endpoint.is_none())
            .ok_or(Error::RegistryFull)?;
        *slot = Some(Endpoint {
            owner,
            ..Endpoint::EMPTY
        });
        Ok(())
    }

    fn unregister(&mut self, owner: u64) -> Result<(), Error> {
        let slot = self
            .endpoints
            .iter_mut()
            .find(|endpoint| endpoint.is_some_and(|endpoint| endpoint.owner == owner))
            .ok_or(Error::NoEndpoint)?;
        *slot = None;
        Ok(())
    }

    fn send(&mut self, sender: u64, receiver: u64, payload: &[u8]) -> Result<(), Error> {
        if payload.len() > MAX_MESSAGE_SIZE {
            return Err(Error::MessageTooLarge);
        }
        if self.endpoint(sender).is_none() {
            return Err(Error::NoEndpoint);
        }
        let endpoint = self.endpoint_mut(receiver).ok_or(Error::NoEndpoint)?;
        let mut message = Message {
            sender,
            ..Message::EMPTY
        };
        message.length = payload.len();
        message.bytes[..payload.len()].copy_from_slice(payload);
        endpoint.enqueue(message)
    }

    fn receive(&mut self, receiver: u64) -> Result<Message, Error> {
        self.endpoint_mut(receiver)
            .ok_or(Error::NoEndpoint)?
            .dequeue()
    }

    fn peek(&self, receiver: u64) -> Result<Message, Error> {
        let endpoint = self.endpoint(receiver).ok_or(Error::NoEndpoint)?;
        endpoint
            .messages
            .first()
            .copied()
            .filter(|_| endpoint.length != 0)
            .ok_or(Error::QueueEmpty)
    }

    fn endpoint(&self, owner: u64) -> Option<&Endpoint> {
        self.endpoints
            .iter()
            .flatten()
            .find(|endpoint| endpoint.owner == owner)
    }

    fn endpoint_mut(&mut self, owner: u64) -> Option<&mut Endpoint> {
        self.endpoints
            .iter_mut()
            .flatten()
            .find(|endpoint| endpoint.owner == owner)
    }
}

static STATE: Mutex<State> = Mutex::new(State::new());

pub fn register(owner: u64) -> Result<(), Error> {
    STATE.lock().register(owner)
}

pub fn unregister(owner: u64) -> Result<(), Error> {
    STATE.lock().unregister(owner)
}

pub fn send(sender: u64, receiver: u64, payload: &[u8]) -> Result<(), Error> {
    STATE.lock().send(sender, receiver, payload)
}

pub fn receive(receiver: u64) -> Result<Message, Error> {
    STATE.lock().receive(receiver)
}

pub fn peek(receiver: u64) -> Result<Message, Error> {
    STATE.lock().peek(receiver)
}

pub fn endpoint_count() -> usize {
    STATE.lock().endpoints.iter().flatten().count()
}

pub fn self_test() -> bool {
    let mut state = State::new();
    let payload = b"wovenhat-ipc";
    if state.register(10).is_err()
        || state.register(20).is_err()
        || state.register(10) != Err(Error::EndpointExists)
        || state.send(10, 20, payload).is_err()
    {
        return false;
    }

    let Ok(message) = state.receive(20) else {
        return false;
    };
    message.sender == 10
        && message.payload() == payload
        && state.receive(20) == Err(Error::QueueEmpty)
        && state.send(99, 20, payload) == Err(Error::NoEndpoint)
        && state.unregister(10).is_ok()
        && state.unregister(10) == Err(Error::NoEndpoint)
}
