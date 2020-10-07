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
        let this = Pin::into_inner(self);
        match this.inner.try_recv() {
            Ok(val) => {
                this.inner.wake_send();
                Poll::Ready(Some(val))
            }
            Err(TryRecvError::Empty) => {
                this.inner.set_recv(ctx.waker());
                match this.inner.try_recv() {
                    Ok(val) => { // sorry for leaving a waker
                        this.inner.wake_send();
                        Poll::Ready(Some(val))
                    }
                    Err(TryRecvError::Empty) => {
                        Poll::Pending
                    }
                    Err(TryRecvError::Closed) => {
                        this.done = true;
                        Poll::Ready(None)
                    }
                }
            }
            Err(TryRecvError::Closed) => {
                this.done = true;
                Poll::Ready(None)
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
