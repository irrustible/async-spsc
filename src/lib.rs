//! A fast, small, full-featured async-aware spsc channel
#![no_std]
extern crate alloc;
use alloc::sync::Arc;
use concurrent_queue::ConcurrentQueue;

mod inner;
pub(crate) use inner::Inner;

mod sender;
pub use sender::Sender;

mod receiver;
pub use receiver::Receiver;

/// Create a new oneshot channel pair.
pub fn bounded<T>(size: usize) -> (Sender<T>, Receiver<T>) {
    let queue = ConcurrentQueue::bounded(size);
    let inner = Arc::new(Inner::new(queue));
    let sender = Sender::new(inner.clone());
    let receiver = Receiver::new(inner);
    (sender, receiver)
}

pub fn unbounded<T>() -> (Sender<T>, Receiver<T>) {
    let queue = ConcurrentQueue::unbounded();
    let inner = Arc::new(Inner::new(queue));
    let sender = Sender::new(inner.clone());
    let receiver = Receiver::new(inner);
    (sender, receiver)
}

/// An empty struct that signifies the channel is closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Closed();

/// We couldn't receive a message.
#[derive(Debug)]
pub enum TryRecvError {
    /// The Sender didn't send us a message yet.
    Empty,
    /// The Sender has dropped.
    Closed,
}

/// We couldn't receive a message.
#[derive(Debug)]
pub enum TrySendError<T> {
    /// The Sender didn't send us a message yet.
    Full(T),
    /// The Sender has dropped.
    Closed(T),
}
