use crate::*;
use core::pin::Pin;
use core::task::{Context, Poll};
use futures_lite::Stream;

/// The receiving half of a oneshot channel.
#[derive(Debug)]
pub struct Receiver<T> {
    inner: Arc<Inner<T>>,
    done: bool,
}

impl<T> Receiver<T> {

    pub(crate) fn new(inner: Arc<Inner<T>>) -> Self {
        Receiver { inner, done: false }
    }

    /// Closes the channel by causing an immediate drop.
    pub fn close(self) { }

    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        self.inner.try_recv()
    }

}

impl<T> Stream for Receiver<T> {
    type Item = T;
    fn poll_next(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Option<T>> {
        match self.inner.try_recv() {
            Ok(val) => {
                self.inner.wake_send();
                Poll::Ready(Some(val))
            }
            Err(TryRecvError::Closed) => Poll::Ready(None),
            Err(TryRecvError::Empty) => {
                self.inner.set_recv(ctx.waker());
                Poll::Pending
            }
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        if !self.done {
            self.inner.close();
            self.inner.wake_send();
        }
    }
}
