//! Fast, easy-to-use, async-aware single-producer/single-consumer
//! (SPSC) channel based around a ringbuffer.
//!
//! # Examples
//!
//! ```
//! use async_spsc::spsc;
//!
//! async fn async_example() {
//!   let (mut sender, mut receiver) = spsc::<i32>(2);
//!   assert!(sender.send(42).await.is_ok());
//!   assert!(sender.send(420).await.is_ok());
//!   assert_eq!(receiver.receive().await, Ok(42));
//!   assert_eq!(receiver.receive().await, Ok(420));
//!   assert!(sender.send(7).await.is_ok());
//!   assert_eq!(receiver.receive().await, Ok(7));
//! }
//!
//! fn sync_example() {
//!   let (mut sender, mut receiver) = spsc::<i32>(2);
//!   assert!(sender.send(42).now().is_ok());
//!   assert!(sender.send(420).now().is_ok());
//!   assert!(sender.send(7).now().is_err()); // no space!
//!
//!   assert_eq!(receiver.receive().now(), Ok(Some(42)));
//!   assert_eq!(receiver.receive().now(), Ok(Some(420)));
//!   assert!(receiver.receive().now().is_err()); // no message!
//!
//!   assert!(sender.send(7).now().is_err());
//!   assert_eq!(receiver.receive().now(), Ok(Some(7)));
//! }
//! ```
#![no_std]

#[cfg(feature="alloc")]
extern crate alloc;

use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr::{NonNull, drop_in_place};
use core::sync::atomic::{AtomicUsize, Ordering};
#[cfg(feature="async")]
use core::{future::Future, pin::Pin, task::{Context, Poll}};

#[cfg(feature="async")]
use atomic_waker::AtomicWaker;

#[cfg(feature="alloc")]
use pages::*;

mod state;
use state::*;
pub mod sender;
pub use sender::*;
pub mod receiver;
pub use receiver::*;

// Sender/Receiver operation-local flags
const WAITING: u8 = 1;

#[derive(Debug)]
enum Holder<'a, 'b, T> {
    /// A pointer we do not own and will not attempt to free.
    BorrowedPtr(NonNull<Spsc<'b, T>>, PhantomData<&'a ()>),
    /// A pointer to a page we manage. This is an owned object we are
    /// abusing, so we need to suppress its destructor and manually
    /// drop it only when both sides are done.
    #[cfg(feature="alloc")]
    Page(PageRef<Atomics, T>),
    // // A pointer produced from [`Box::leak`] that's potentially
    // // shared with other holders.
    // #[cfg(feature="alloc")]
    // SharedBoxPtr(NonNull<Spsc<'b, T>>),
}

impl<'a, 'b, T> Clone for Holder<'a, 'b, T> {
    fn clone(&self) -> Self {
        match self {
            Holder::BorrowedPtr(r, p) => Holder::BorrowedPtr(*r, *p),
            #[cfg(feature="alloc")]
            Holder::Page(r) => Holder::Page(*r),
            // #[cfg(feature="alloc")]
            // Holder::SharedBoxPtr(r) => Holder::SharedBoxPtr(*r),
        }
    }
}

impl<'a, 'b, T> Copy for Holder<'a, 'b, T> {}

impl<'a, 'b, T> Holder<'a, 'b, T> {

    #[inline(always)]
    fn atomics(&self) -> *const Atomics {
        match self {
            Holder::BorrowedPtr(r, _) => &unsafe { r.as_ref() }.atomics,
            Holder::Page(p) => unsafe { p.header() }
        }
    }

    #[inline(always)]
    fn data(&mut self) -> *mut MaybeUninit<T> {
        match self {
            Holder::BorrowedPtr(r, _) => unsafe { r.as_ref() }.data(),
            Holder::Page(p) => unsafe { p.data() },
        }
    }

    // Safe only if we are the last referent to the spsc.
    unsafe fn cleanup(self, capacity: Half, state: State) {
        // whatever we are, we are going to drop the inflight items
        // and the wakers if there are any.
        match self {
            Holder::BorrowedPtr(ptr, _) => 
                ptr.as_ref().cleanup(capacity, state),
            #[cfg(feature="alloc")]
            Holder::Page(c) => {
                drop_in_flight(c.data(), capacity, state);
                PageRef::drop(c);
            }
        }
    }

    // // Safe only if we are the last active referent
    // pub(crate) unsafe fn recycle(self) {
    //     (*self.inner.get()).reset();
    //     self.flags.store(0, orderings::STORE);
    // }
}

fn drop_in_flight<T>(items: *mut MaybeUninit<T>, capacity: Half, state: State) {
    // TODO: probably not optimal
    let front = state.front().position();
    let mut back = state.back();
    loop {
        let b = back.position();
        if front == b { break; }
        let index = (b % capacity) as usize;
        unsafe { drop_in_place(items.add(index));  }
        back = back.advance(capacity, 1);
    }
}

/// Creates a new heap-backed [`Spsc`] that can store up to `capacity`
/// in-flight messages at a time.
pub fn spsc<T>(capacity: Half) -> (Sender<'static, 'static, T>, Receiver<'static, 'static, T>) {
    // First we must check we can handle this capacity.
    assert!(capacity > 0);
    assert!(capacity <= MAX_CAPACITY);
    let page = PageRef::new(Atomics::default(), capacity);
    let holder = Holder::Page(page);
    (Sender::new(holder, State(0), capacity), Receiver::new(holder, State(0), capacity))
}

#[derive(Debug)]
pub struct Spsc<'a, T> {
    atomics:  Atomics,
    ptr:      NonNull<MaybeUninit<T>>,
    capacity: Half,
    _phantom: PhantomData<&'a T>
}

impl<'a, T> Spsc<'a, T> {
    fn cleanup(&self, capacity: Half, state: State) {
        // Safe because we have exclusive access
        drop_in_flight(self.data(), capacity, state);
        // Avoid a potential memory leak.
        self.atomics.drop_wakers();
    }

    fn data(&self) -> *mut MaybeUninit<T> { self.ptr.as_ptr() }
}

impl<'a, T> Spsc<'a, T> {
    // /// ## Safety
    // ///
    // /// * ptr must point to a len-sized array of appropriately aligned
    // ///   and padded T which should already be initialised.
    // ///
    // /// Note: will panic if length is 0 or greater than can be
    // /// represented in two bits less than half a usize.
    // pub unsafe fn from_nonnull_len(ptr: NonNull<MaybeUninit<T>>, len: Half) -> Self {
    //     assert!(len > 0, "the spsc buffer must have a non-zero length");
    //     assert!(len <= MAX_CAPACITY, "the spsc buffer must have a length representable in two bits less than half a usize")
    //     Spsc();
    //     Self::make(Slice::BorrowedPtrLen(ptr, len))
    // }

//     // /// ## Safety
//     // ///
//     // /// * len must not be zero
//     // /// * ptr must point to a len-sized array of appropriately aligned
//     // ///   and padded T which should already be initialised.
//     // ///
//     // /// Note: will panic if length is 0 or greater than can be
//     // /// represented in two bits less than half a usize.
//     // pub unsafe fn from_raw_parts(ptr: *mut MaybeUninit<T>, len: Half) -> Self {
//     //     assert!(len > 0, "the spsc buffer must have a non-zero length");
//     //     assert!(len <= MAX_CAPACITY, "the spsc buffer must have a length representable in two bits less than half a usize");
//     //     Self::make(Slice::BorrowedPtrLen(NonNull::new_unchecked(ptr), len))
//     // }

}

#[derive(Debug,Default)]
pub struct Atomics {
    state:    AtomicUsize,
    #[cfg(feature="async")]
    sender:   AtomicWaker,
    #[cfg(feature="async")]
    receiver: AtomicWaker,
}

impl Atomics {
    fn drop_wakers(&self) {
        let _s = self.sender.take();
        let _r = self.receiver.take();
    }
}

#[derive(Debug,Eq,Hash,PartialEq)]
pub struct SendError<T> {
    pub kind:  SendErrorKind,
    pub value: T,
}
#[derive(Debug,Eq,Hash,PartialEq)]
pub enum SendErrorKind {
    Closed,
    Full,
}

#[derive(Debug,Eq,Hash,PartialEq)]
pub struct Closed;
