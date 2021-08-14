use crate::*;
use core::cell::Cell;

pub struct Sender<'a, 'b, T> {
    spsc:  Option<Holder<'a, 'b, T>>,
    state: Cell<State>,
    cap:   Half,
}

impl<'a, 'b, T> Sender<'a, 'b, T> {

    pub(super) fn new(spsc: Holder<'a, 'b, T>, state: State, cap: Half) -> Self {
        Sender { spsc: Some(spsc), state: Cell::new(state), cap }
    }

    /// Indicates how many send slots are known to be available.
    ///
    /// Note: this checks our local cache of the state, so the true
    /// figure may be greater. We will find out when we next send.
    pub fn space(&self) -> Half { self.state.get().space(self.cap) }

    /// Indicates whether we believe there to be no space left to send.
    ///
    /// Note: this checks our local cache of the state, so the true
    /// figure may be greater. We will find out when we next send.
    pub fn is_full(&self) -> bool { self.state.get().is_full(self.cap) }

    /// Indicates whether the channel is empty.
    pub fn is_empty(&self) -> bool { self.state.get().is_full(self.cap) }

    /// Indicates the capacity of the channel, the maximum number of
    /// messages that can be in flight at a time.
    pub fn capacity(&self) -> Half { self.cap }

    pub fn send<'c>(&'c mut self, value: T) -> Sending<'c, 'a, 'b, T> {
        Sending { sender: Some(self),value: Some(value), flags: 0 }
    }

    // pub fn batch<'c>(&'c mut self) -> Batch<'c, 'a, 'b, T> {
    //     let state = self.state;
    //     Batch { sender: Some(self), state }
    // }

}
impl<'a, 'b, T> Drop for Sender<'a, 'b, T> {
    fn drop(&mut self) {
        if let Some(spsc) = self.spsc.take() {
            let state = self.state.get();
            if state.is_closed() {
                unsafe { spsc.cleanup(self.cap, state); }
                return;
            }
            let atomics = unsafe { &*spsc.atomics() };
            let state = State(atomics.state.fetch_xor(S_CLOSE, Ordering::AcqRel));
            if state.is_closed() {
                unsafe { spsc.cleanup(self.cap, state); }
            } else {
                atomics.receiver.wake();
            }
        }
    }
}

unsafe impl<'a, 'b, T: Send> Send for Sender<'a, 'b, T> {}
unsafe impl<'a, 'b, T: Send> Sync for Sender<'a, 'b, T> {}

/// Sends a single message.
pub struct Sending<'a, 'b, 'c, T> {
    sender: Option<&'a mut Sender<'b, 'c, T>>,
    value:  Option<T>,
    flags:  u8,
}

fn closed<T>(value: T) -> Result<(), SendError<T>> {
    Err(SendError { kind: SendErrorKind::Closed, value })
}

fn full<T>(value: T) -> Result<(), SendError<T>> {
    Err(SendError { kind: SendErrorKind::Full, value })
}

impl<'a, 'b, 'c, T> Sending<'a, 'b, 'c, T> {
    pub fn now(mut self) -> Result<(), SendError<T>> {
        let sender = self.sender.take().unwrap();
        let value = self.value.take().unwrap();
        if let Some(spsc) = sender.spsc.as_mut() {
            let cap = sender.cap;
            let mut state = sender.state.get();
            // We do nothing if we're closed.
            if state.is_closed() { return closed(value); }
            if state.is_full(cap) {
                // The Receiver may have cleared space since the cache
                // was last updated; refresh and recheck.
                state = State(unsafe { &*spsc.atomics() }.state.load(Ordering::Acquire));
                sender.state.set(state);
                if state.is_closed() { return closed(value); }
                if state.is_full(cap) { return full(value); }
            }
            // Still here? Cool, we can write the value now.
            let s = state.front();
            unsafe { spsc.data().add(s.index(cap)).write(MaybeUninit::new(value)) };
            // Update the atomic with our advance.
            let mask = (s.0 ^ s.advance(cap, 1).0) as usize;
            let atomics = unsafe { &* spsc.atomics() };
            let state2 = State(atomics.state.fetch_xor(mask, Ordering::Acquire) ^ mask);
            sender.state.set(state2);
            if state2.is_closed() {
                // Oh. Well we need our item back for the SendError.
                let value = unsafe { spsc.data().add(s.index(cap)).read().assume_init() };
                // We already committed our advance. To avoid double
                // freeing, we have to wind back the sender's local
                // cache of the state in lieu of an atomic op.
                sender.state.set(State((state.0 & BACK) | s.0 as usize));
                return closed(value);
            }
            // Before we go, let the receiver know there's a message.
            #[cfg(feature="async")]
            atomics.receiver.wake();
            return Ok(());
        }
        closed(value)
    }
}

#[cfg(feature="async")]
impl<'a, 'b, 'c, T> Future for Sending<'a, 'b, 'c, T> {
    type Output = Result<(), SendError<T>>;
    fn poll(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        let this = unsafe { Pin::get_unchecked_mut(self) };
        let sender = this.sender.take().unwrap();
        let value = this.value.take().unwrap();
        if let Some(spsc) = sender.spsc.as_mut() {
            let cap = sender.cap;
            let mut state = sender.state.get();
            // First we must check we're not closed.
            if state.is_closed() { return Poll::Ready(closed(value)); }
            // Try to find space without hitting the atomic.
            if state.is_full(cap) {
                let atomics = unsafe { &*spsc.atomics() };
                state = State(atomics.state.load(Ordering::Acquire));
                sender.state.set(state);
                // We have to check again because of that refresh.
                if state.is_closed() { return Poll::Ready(closed(value)); }
                if state.is_full(cap) {
                    // We'll have to wait.
                    this.flags |= WAITING;
                    atomics.sender.register(ctx.waker());
                    // We'll also have to put ourselves back.
                    this.sender.replace(sender);
                    this.value.replace(value);
                    return Poll::Pending
                }
            }
            // Still here? Cool, we can write the value now.
            let s = state.front();
            unsafe { spsc.data().add(s.index(cap)).write(MaybeUninit::new(value)) };
            // Update the atomic with our advance.
            let mask = (s.0 ^ s.advance(cap, 1).0) as usize;
            let atomics = unsafe { &*spsc.atomics() };
            let state2 = State(atomics.state.fetch_xor(mask, Ordering::Acquire) ^ mask);
            sender.state.set(state2);
            if state2.is_closed() {
                // Oh. Well we need our item back for the SendError.
                let value = unsafe { spsc.data().add(s.index(cap)).read().assume_init() };
                // We already committed our advance. To avoid double
                // freeing, we have to wind back the sender's local
                // cache of the state in lieu of an atomic op.
                sender.state.set(State((state.0 & BACK) | s.0 as usize));
                return Poll::Ready(closed(value));
            }
            // Before we go, let the receiver know there's a message.
            atomics.receiver.wake();
            return Poll::Ready(Ok(()));
        }
        Poll::Ready(closed(value))
    }
}

impl<'a, 'b, 'c, T> Drop for Sending<'a, 'b, 'c, T> {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            if (self.flags & WAITING) != 0 {
                // We left a waker we should probably clear up
                sender.spsc.as_mut().map(|r| unsafe { &*r.atomics() }.sender.take());
            }
        }
    }
}

// /// 
// pub struct Batch<'a, 'b, 'c, T> {
//     sender: Option<&'a mut Sender<'b, 'c, T>>,
//     state: State,
// }

// impl<'a, 'b, 'c, T> Batch<'a, 'b, 'c, T> {
//     pub fn space(&self) -> Half {
//         let sender = self.sender.as_ref().unwrap();
//         self.state.space(sender.cap)
//     }
//     // pub fn push(&mut self, value: T) -> Result<(), SendError<T>> {
//     //     Ok(())
//     // }
// }

// #[cfg(feature="async")]
// impl<'a, 'b, 'c, T> Future for Batch<'a, 'b, 'c, T> {
//     type Output = Result<(), SendError<()>>;
//     fn poll(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
//         todo!()
//     }
// }

// impl<'a, 'b, 'c, T> Drop for Batch<'a, 'b, 'c, T> {
//     fn drop(&mut self) {
//         if let Some(sender) = self.sender.take() {
//             let front = self.state.front();
//             let sender_front = sender.state.front();
//             // Check if we have to do anything.
//             if front != sender_front {
//                 if let Some(spsc) = sender.spsc {
//                     // We have some uncommitted changes, we should propagate them.
//                     let mask = (front.0 ^ sender_front.0) as usize;
//                     let state = State(spsc.atomics.state.fetch_xor(mask, Ordering::AcqRel));
//                     if !state.is_closed() {
//                         // propagate the updated state to the sender.
//                         sender.state = State(state.0 ^ mask);
//                     }
//                 }
//             }
//         }
//     }
// }

