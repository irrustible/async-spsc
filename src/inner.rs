use core::task::Waker;
use crate::*;
use atomic_waker::AtomicWaker;
use concurrent_queue::{ConcurrentQueue, PopError, PushError};

#[derive(Debug)]
pub struct Inner<T> {
    queue: ConcurrentQueue<T>,
    send: AtomicWaker,
    recv: AtomicWaker,
}

impl<T> Inner<T> {
    #[inline]
    pub(crate) fn new(queue: ConcurrentQueue<T>) -> Self {
        Inner {
            queue,
            send: AtomicWaker::new(),
            recv: AtomicWaker::new(),
        }
    }

    #[inline]
    pub(crate) fn close(&self) {
        self.queue.close();
    }

    #[inline]
    pub(crate) fn is_closed(&self) -> bool {
        self.queue.is_closed()
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    #[inline]
    pub(crate) fn is_full(&self) -> bool {
        self.queue.is_full()
    }

    #[inline]
    pub(crate) fn try_recv(&self) -> Result<T, TryRecvError> {
        match self.queue.pop() {
            Ok(r) => Ok(r),
            Err(PopError::Empty) => Err(TryRecvError::Empty),
            Err(PopError::Closed) => Err(TryRecvError::Closed),
        }
    }

    #[inline]
    pub(crate) fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        match self.queue.push(value) {
            Ok(()) => Ok(()),
            Err(PushError::Full(v)) => Err(TrySendError::Full(v)),
            Err(PushError::Closed(v)) => Err(TrySendError::Closed(v)),
        }
    }

    #[inline]
    pub(crate) fn set_send(&self, waker: &Waker) { self.send.register(waker) }

    #[inline]
    pub(crate) fn set_recv(&self, waker: &Waker) { self.recv.register(waker) }

    #[inline]
    pub(crate) fn wake_send(&self) { self.send.wake() }

    #[inline]
    pub(crate) fn wake_recv(&self) { self.recv.wake() }
}

unsafe impl<T: Send> Send for Inner<T> {}
unsafe impl<T: Sync> Sync for Inner<T> {}
