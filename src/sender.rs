use crate::*;
use alloc::sync::Arc;
use core::task::Poll;
use core::future::Future;
use futures_micro::*;
// use futures_micro::poll_state;

/// The sending half of a oneshot channel.
#[derive(Debug)]
pub struct Sender<T> {
    inner: Arc<Inner<T>>,
    done: bool,
}

impl<T> Sender<T> {

    #[inline]
    pub(crate) fn new(inner: Arc<Inner<T>>) -> Self {
        Sender { inner, done: false }
    }
        
    #[inline]
    pub fn try_send(&mut self, value: T) -> Result<(), TrySendError<T>> {
        if self.done {
            Err(TrySendError::Closed(value))
        } else {
            let ret = self.inner.try_send(value);
            if let Err(TrySendError::Closed(_)) = ret {
                self.done = true;
            }
            ret
        }
    }

    /// Closes the channel by causing an immediate drop
    #[inline]
    pub fn close(self) { }

    /// true if the channel is closed
    #[inline]
    pub fn is_closed(&self) -> bool { self.inner.is_closed() }

    /// true if the channel is empty
    #[inline]
    pub fn is_empty(&self) -> bool { self.inner.is_empty() }

    /// true if the channel is full
    #[inline]
    pub fn is_full(&self) -> bool { self.inner.is_full() }

    /// Sends a message on the channel. Fails if the Receiver is dropped.
    #[inline]
    pub fn send<'a>(&'a mut self, value: T) -> impl Future<Output=Result<(), Closed>> + 'a {
        poll_state(Some(value), move |opt, ctx| {
            match self.inner.try_send(opt.take().unwrap()) {
                Ok(()) => {
                    self.inner.wake_recv();
                    Poll::Ready(Ok(()))
                }
                Err(TrySendError::Full(val)) => {
                    self.inner.set_send(ctx.waker());
                    match self.inner.try_send(val) {
                        Ok(()) => { // sorry for leaving a waker
                            self.inner.wake_recv();
                            Poll::Ready(Ok(()))
                        }
                        Err(TrySendError::Full(val)) => {
                            *opt = Some(val);
                            Poll::Pending
                        }
                        Err(TrySendError::Closed(_)) => {
                            self.done = true;
                            Poll::Ready(Err(Closed()))
                        }
                    }
                }
                Err(TrySendError::Closed(_)) => {
                    self.done = true;
                    Poll::Ready(Err(Closed()))
                }
            }
        })
    }        
}

impl<T> Drop for Sender<T> {
    #[inline]
    fn drop(&mut self) {
        if !self.done {
            self.inner.close();
            self.inner.wake_recv();
        }
    }
}
