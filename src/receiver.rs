use super::*;
// #[cfg(feature="stream")]
// use futures_core::stream::Stream;

pub struct Receiver<'a, 'b, T> {
    spsc:  Option<Holder<'a, 'b, T>>,
    state: State,
    cap:   Half,
}

impl<'a, 'b, T> Receiver<'a, 'b, T> {

    pub(super) fn new(spsc: Holder<'a, 'b, T>, state: State, cap: Half) -> Self {
        Receiver { spsc: Some(spsc), state, cap }
    }

    fn refresh_state(&mut self) -> State {
        let spsc = self.spsc.as_mut().unwrap();
        self.state = State(spsc.atomics.state.load(Ordering::Acquire));
        self.state
    }

    fn update_state(&mut self, mask: Half) -> State {
        let mask = (mask as usize) << BITS;
        let spsc = self.spsc.as_mut().unwrap();
        self.state = State(spsc.atomics.state.fetch_xor(mask, Ordering::Acquire) ^ mask);
        self.state
    }

    /// Returns a disposable object which can receive a single message
    /// either synchronously via [`Receiving::now`] or asynchronously
    /// via the [`core::future::Future`] instance.
    pub fn receive<'c>(&'c mut self) -> Receiving<'a, 'b, 'c, T> {
        Receiving { receiver: Some(self) }
    }
}

unsafe impl<'a, 'b, T: Send> Send for Receiver<'a, 'b, T> {}
unsafe impl<'a, 'b, T: Send> Sync for Receiver<'a, 'b, T> {}

impl<'a, 'b, T> Drop for Receiver<'a, 'b, T> {
    fn drop(&mut self) {
        if let Some(spsc) = self.spsc.take() {
            // If we already know they've closed, clean up.
            if self.state.is_closed() {
                unsafe { spsc.cleanup(self.cap, self.state); }
                return;
            }
            // Mark ourselves closed
            let state = State(spsc.atomics.state.fetch_xor(R_CLOSE, Ordering::AcqRel));
            if state.is_closed() {
                // We were beaten to it.
                unsafe { spsc.cleanup(self.cap, self.state); }
            } else {
                // We should wake them
                spsc.atomics.sender.wake();
            }
        }
    }
}

/// A single Receive operation that can be performed synchronously
/// (with [`Receiving::now`]) or asynchronously (with the
/// [`core::future::Future`] instance).
pub struct Receiving<'a, 'b, 'c, T> {
    receiver: Option<&'c mut Receiver<'a, 'b, T>>,
}

impl<'a, 'b, 'c, T> Receiving<'a, 'b, 'c, T> {
    pub fn now(mut self) -> Result<Option<T>, Closed> {
        // Take our receiver, since we can't be called again.
        let receiver = self.receiver.take().unwrap();
        if let Some(spsc) = receiver.spsc {
            // We are going to first check our local cached state. If
            // it tells us there is space, we don't need to
            // synchronise to receive!
            let mut state = receiver.state;
            // The Receiver is slightly different logic to the Sender
            // since if there are still messages in flight, we can
            // receive them even if the Sender closed. Thus if we hit
            // a close, having already taken our local receiver,
            // there's nothing to do in terms of cleanup.
            if state.is_empty() {
                if state.is_closed() { return Err(Closed); }
                // Hard luck, time to synchronise (and recheck)
                state = receiver.refresh_state();
                if state.is_empty() {
                    if state.is_closed() { return Err(Closed); }
                    return Ok(None);
                }
            }
            // Still here? Fabulous, we have a message waiting for us.
            let back = receiver.state.back();
            // This mouthful takes the value, leaving the slot uninitialised
            let value = unsafe {
                (&mut *spsc.buffer.get())[back.position()].as_mut_ptr().read()
            };
            // Now inform the Sender they can have this slot back.
            let b = back.advance(receiver.cap, 1);
            let state = receiver.update_state(back.0 ^ b.0);
            // Now we attempt to wake the Sender if they are not
            // closed. There will probably be nothing here.
            #[cfg(feature="async")]
            if !state.is_closed() { spsc.atomics.sender.wake(); }
            return Ok(Some(value));
        }
        Err(Closed)
    }
}

#[cfg(feature="async")]
impl<'a, 'b, 'c, T> Future for Receiving<'a, 'b, 'c, T> {
    type Output = Result<T, Closed>;
    fn poll(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        let this = unsafe { Pin::get_unchecked_mut(self) };
        let receiver = this.receiver.take().unwrap();
        if let Some(spsc) = receiver.spsc {
            let cap = receiver.cap;
            let mut state = receiver.state;
            // Try to find a message without hitting the atomic.
            if state.is_empty() {
                // If we're closed, we don't need to synchronise again.
                if state.is_closed() { return Poll::Ready(Err(Closed)); }
                // No? let's refresh the state then and check again
                state = receiver.refresh_state();
                if state.is_empty() {
                    if state.is_closed() { return Poll::Ready(Err(Closed)); }
                    // Go into hibernation
                    spsc.atomics.receiver.register(ctx.waker());
                    this.receiver.replace(receiver);
                    return Poll::Pending;
                }
            }
            // Good news, we can receive a value.
            let back = receiver.state.back();
            let value = unsafe { (&mut *spsc.buffer.get())[back.position()].as_mut_ptr().read() };
            // Now inform the other side we're done reading.
            let b = back.advance(cap, 1);
            let state = receiver.update_state(back.0 ^ b.0);
            // Now we attempt to wake the Sender if they are not
            // closed. There will probably be nothing here.
            if !state.is_closed() { spsc.atomics.sender.wake(); }
            return Poll::Ready(Ok(value));
        }
        Poll::Ready(Err(Closed))
    }
}

// pub struct Batch<'a, 'b, 'c, T> {
//     receiver: Option<&'a mut Receiver<'b, 'c, T>>,
//     state:    State,
// }

// // impl<'a, 'b, 'c, T> Future for Batch<'a, 'b, 'c, T> {
// //     type Output = Result<Self, Closed>;
// //     fn poll(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
// //         let this = unsafe { Pin::get_unchecked_mut(self) };
// //         if let Some(sender) = self.sender.take() {
// //             if self.state.is_empty() {
// //                 if state.is_closed() { return Poll::Ready(Err(Closed)); }
// //                 if self.state == sender.state {
// //                 }
// //             }
// //         }
// //     }
// // }

// impl<'a, 'b, 'c, T> Iterator for Batch<'a, 'b, 'c, T> {
//     type Item = T;
//     fn next(&mut self) -> Option<T> {
//         if let Some(receiver) = self.receiver.as_mut() {
//             if let Some(spsc) = receiver.spsc {
//                 // The batch only fetches items that are known to be
//                 // available for reading already.
//                 let state = self.state;
//                 if state.is_empty() { return None; }
//                 // Still here? Fabulous, we have a message waiting for us.
//                 let back = state.back();
//                 let value = unsafe {
//                     (&mut *spsc.buffer.get())[back.position()].as_mut_ptr().read()
//                 };
//                 // Update our local version of the state.
//                 self.state = state.with_back(back.advance(receiver.cap, 1));
//                 return Some(value);
//             }
//         }
//         None
//     }
// }

// impl<'a, 'b, 'c, T> Drop for Batch<'a, 'b, 'c, T> {
//     fn drop(&mut self) {
//         if let Some(receiver) = self.receiver.take() {
//             if receiver.spsc.is_some() {
//                 // We need to update the Receiver's state cache if the other side isn't closed 
//                 if self.state != receiver.state {
//                     if self.state.is_closed() {
//                         // no point updating the atomic, just update the receiver
//                         receiver.state = self.state;
//                     } else {
//                         // apply our changes to the atomic and the receiver.
//                         let mask = receiver.state.front().0 ^ self.state.front().0;
//                         receiver.update_state(mask);
//                     }
//                 }
//             }
//         }
//     }
// }
