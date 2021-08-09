use super::*;

pub struct Sender<'a, 'b, T> {
    spsc:  Option<Holder<'a, 'b, T>>,
    state: State,
    cap:   Half,
}

impl<'a, 'b, T> Sender<'a, 'b, T> {

    pub(super) fn new(spsc: Holder<'a, 'b, T>, state: State, cap: Half) -> Self {
        Sender { spsc: Some(spsc), state, cap }
    }

    /// Indicates how many send slots are known to be available.
    ///
    /// Note: this checks our local cache of the state, so the true
    /// figure may be greater. We will find out when we next send.
    pub fn space(&self) -> Half { self.state.space(self.cap) }

    /// Indicates whether we believe there to be no space left to send.
    ///
    /// Note: this checks our local cache of the state, so the true
    /// figure may be greater. We will find out when we next send.
    pub fn is_full(&self) -> bool { self.state.is_full(self.cap) }

    /// Indicates whether the channel is empty.
    pub fn is_empty(&self) -> bool { self.state.is_full(self.cap) }

    /// Indicates the capacity of the channel, the maximum number of
    /// messages that can be in flight at a time.
    pub fn capacity(&self) -> Half { self.cap }

    pub fn send<'c>(&'c mut self, value: T) -> Sending<'c, 'a, 'b, T> {
        Sending { sender: Some(self), value: Some(value), flags: 0 }
    }

    // pub fn batch<'c>(&'c mut self) -> Batch<'c, 'a, 'b, T> {
    //     let state = self.state;
    //     Batch { sender: Some(self), state }
    // }

    fn refresh_state(&mut self) -> State {
        let spsc = self.spsc.as_mut().unwrap();
        self.state = State(spsc.atomics.state.load(Ordering::Acquire));
        self.state
    }

    fn update_state(&mut self, mask: Half) -> State {
        let mask = mask as usize;
        let spsc = self.spsc.as_mut().unwrap();
        self.state = State(spsc.atomics.state.fetch_xor(mask, Ordering::Acquire) ^ mask);
        self.state
    }

    // Called when we have notified the receiver of a message we have
    // sent, only to learn that they have closed. When we do a close,
    // we will use the modified state, even though we haven't updated
    // the atomic.
    fn rewind_state(&mut self, to: HalfState) {
        self.state = State((self.state.0 & BACK) | to.0 as usize);
    }
}

impl<'a, 'b, T> Drop for Sender<'a, 'b, T> {
    fn drop(&mut self) {
        if let Some(spsc) = self.spsc.take() {
            if self.state.is_closed() {
                unsafe { spsc.cleanup(self.cap, self.state); }
                return;
            }                
            let state = State(spsc.atomics.state.fetch_xor(S_CLOSE, Ordering::AcqRel));
            if state.is_closed() {
                unsafe { spsc.cleanup(self.cap, state); }
            } else {
                spsc.atomics.receiver.wake();
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
        if let Some(spsc) = sender.spsc {
            let cap = sender.cap;
            let mut state = sender.state;
            // We do nothing if we're closed.
            if state.is_closed() { return closed(value); }
            if state.is_full(cap) {
                // The Receiver may have cleared space since the cache
                // was last updated; refresh and recheck.
                state = sender.refresh_state();
                if state.is_closed() { return closed(value); }
                if state.is_full(cap) { return full(value); }
            }
            // Still here? Cool, we can write the value now.
            let s = sender.state.front();
            {
                let slot = unsafe { spsc.buffer.get().as_mut().unwrap() };
                slot[s.position()] = MaybeUninit::new(value);
            }
            // Update the atomic with our advance.
            let state2 = sender.update_state(s.0 ^ s.advance(cap, 1).0);
            if state2.is_closed() {
                // Oh. Well we need our item back for the SendError.
                let value = unsafe {
                    (&mut *spsc.buffer.get())[s.position()].as_mut_ptr().read()
                };
                // We already committed our advance. To avoid double
                // freeing, we have to wind back the sender's local
                // cache of the state in lieu of an atomic op.
                sender.rewind_state(s);
                return closed(value);
            }
            // Before we go, let the receiver know there's a message.
            #[cfg(feature="async")]
            spsc.atomics.receiver.wake();
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
        if let Some(spsc) = sender.spsc {
            let cap = sender.cap;
            let mut state = sender.state;
            // First we must check we're not closed.
            if state.is_closed() { return Poll::Ready(closed(value)); }
            // Try to find space without hitting the atomic.
            if state.is_full(cap) {
                state = sender.refresh_state();
                // We have to check again because of that refresh.
                if state.is_closed() { return Poll::Ready(closed(value)); }
                if state.is_full(cap) {
                    // We'll have to wait.
                    this.flags |= WAITING;
                    spsc.atomics.sender.register(ctx.waker());
                    // We'll also have to put ourselves back.
                    this.sender.replace(sender);
                    this.value.replace(value);
                    return Poll::Pending
                }
            }
            // Still here? Cool, we can write the value now.
            let s = sender.state.front();
            {
                let slot = unsafe { spsc.buffer.get().as_mut().unwrap() };
                slot[s.position()] = MaybeUninit::new(value);
            }
            // Update the atomic with our advance.
            let state2 = sender.update_state(s.0 ^ s.advance(cap, 1).0);
            if state2.is_closed() {
                // Oh. Well we need our item back for the SendError.
                let value = unsafe {
                    (&mut *spsc.buffer.get())[s.position()].as_mut_ptr().read()
                };
                // We already committed our advance. To avoid double
                // freeing, we have to wind back the sender's local
                // cache of the state in lieu of an atomic op.
                sender.rewind_state(s);
                return Poll::Ready(closed(value));
            }
            // Before we go, let the receiver know there's a message.
            spsc.atomics.receiver.wake();
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
                sender.spsc.as_mut().map(|r| r.atomics.sender.take());
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

