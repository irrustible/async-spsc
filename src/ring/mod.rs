//! A single-producer, single-consumer channel based around a
//! ringbuffer.
//!
//! # Examples
//!
//! ```
//! use async_spsc::ring::ring;
//!
//! async fn async_example() {
//!   let (mut sender, mut receiver) = ring::<i32>(2);
//!   assert!(sender.send(42).await.is_ok());
//!   assert!(sender.send(420).await.is_ok());
//!   assert_eq!(receiver.receive().await, Ok(42));
//!   assert_eq!(receiver.receive().await, Ok(420));
//!   assert!(sender.send(7).await.is_ok());
//!   assert_eq!(receiver.receive().await, Ok(7));
//! }
//!
//! fn sync_example() {
//!   let (mut sender, mut receiver) = ring::<i32>(2);
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

use core::cell::UnsafeCell;
use core::convert::TryInto;
use core::marker::PhantomData;
// use core::fmt;
use core::mem::MaybeUninit;
use core::ops::Deref;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering};
#[cfg(feature="async")]
use core::{future::Future, pin::Pin, task::{Context, Poll}};
#[cfg(feature="alloc")]
use alloc::alloc::{alloc, dealloc, Layout, LayoutError};

#[cfg(feature="async")]
use atomic_waker::AtomicWaker;

use crate::*;
mod state;
use state::*;
mod slice;
use slice::*;
pub mod sender;
pub use sender::*;
pub mod receiver;
pub use receiver::*;

// Sender/Receiver operation-local flags
const WAITING: u8 = 1;

#[derive(Debug)]
enum Holder<'a, 'b, T> {
    // Lifetime-bound reference. Unable to free
    Ref(&'a Ring<'b, T>),
    // A pointer we do not own and will not attempt to free.
    BorrowedPtr(NonNull<Ring<'b, T>>),
    // A pointer we manage with our contiguous layout strategy.
    #[cfg(feature="alloc")]
    Contiguous(NonNull<Ring<'b, T>>),
    // // A pointer produced from [`Box::leak`] that's potentially
    // // shared with other holders.
    // #[cfg(feature="alloc")]
    // SharedBoxPtr(NonNull<Ring<'b, T>>),
}

impl<'a, 'b, T> Clone for Holder<'a, 'b, T> {
    fn clone(&self) -> Self {
        match self {
            Holder::Ref(r) => Holder::Ref(r),
            Holder::BorrowedPtr(r) => Holder::BorrowedPtr(*r),
            #[cfg(feature="alloc")]
            Holder::Contiguous(r) => Holder::Contiguous(*r),
            // #[cfg(feature="alloc")]
            // Holder::SharedBoxPtr(r) => Holder::SharedBoxPtr(*r),
        }
    }
}

impl<'a, 'b, T> Copy for Holder<'a, 'b, T> {}

impl<'a, 'b, T> Deref for Holder<'a, 'b, T> {
    type Target = Ring<'b, T>;
    fn deref(&self) -> &Ring<'b, T> {
        match self {
            Holder::Ref(b) => b,
            Holder::BorrowedPtr(ptr)  => unsafe { ptr.as_ref() },
            #[cfg(feature="alloc")]
            Holder::Contiguous(c) => unsafe { c.as_ref() },
            // #[cfg(feature="alloc")]
            // Holder::SharedBoxPtr(ptr) => unsafe { ptr.as_ref() },
        }
    }
}

impl<'a, 'b, T> Holder<'a, 'b, T> {

    // Safe only if we are the last referent to the ring.
    unsafe fn cleanup(self, cap: Half, state: State) {
        // whatever we are, we are going to drop the inflight items
        // and the wakers if there are any.
        match self {
            Holder::Ref(r) => r.cleanup(state),
            Holder::BorrowedPtr(ptr) => ptr.as_ref().cleanup(state),
            #[cfg(feature="alloc")]
            Holder::Contiguous(c) => {
                c.as_ref().cleanup(state);
                // We also need to free the ring
                let layout = ContiguousLayout::for_capacity::<T>(cap).unwrap();
                dealloc(c.as_ptr().cast(), layout.layout);
            }
        }
    }

    // // Safe only if we are the last active referent
    // pub(crate) unsafe fn recycle(self) {
    //     (*self.inner.get()).reset();
    //     self.flags.store(0, orderings::STORE);
    // }
}

/// Creates a new heap-backed [`Ring`] that can store up to `capacity`
/// in-flight messages at a time.

// This is a bit horrific, to be frank. The idea is to allocate data
// and ring in a continuous block instead of each individually, thus
// only requiring one alloc/free instead of two.

pub fn ring<T>(capacity: Half) -> (Sender<'static, 'static, T>, Receiver<'static, 'static, T>) {
    // First we must check we can handle this capacity.
    assert!(capacity > 0);
    assert!(capacity <= MAX_CAPACITY);
    // First we get our pertinent layout details and allocate the raw data.
    let layout = ContiguousLayout::for_capacity::<T>(capacity).unwrap();
    let raw = unsafe { alloc(layout.layout) };
    // Now we'll synthesise the data slice.
    let data: *mut MaybeUninit<T> = unsafe { raw.add(layout.data) }.cast();
    let data = unsafe { core::slice::from_raw_parts_mut(data, capacity as usize) };
    // Now initialise the ring.
    let ring: *mut Ring<T> = raw.cast();
    unsafe { ring.write(Ring::make(Slice::Ref(data))); }
    // Now we can create a holder and use it to create both sides.
    let ring = Holder::Contiguous(unsafe { NonNull::new_unchecked(ring) });
    (Sender::new(ring, State(0), capacity), Receiver::new(ring, State(0), capacity))
}

#[repr(C)] // very important for knowing the layout
#[derive(Debug)]
pub struct Ring<'a, T> {
    // Atomics is likely to fit in a cache line:
    //
    // * 2x AtomicWaker @ 3 words = 6 words
    // * 1x atomicusize @ 1 word = 7 words.
    //
    // Buffer is alas 2 words, though we might squeeze it down either
    // by reimplementing AtomicWaker to support two wakers or by using ointers.
    //
    atomics:  Atomics,
    buffer:   UnsafeCell<Slice<'a, T>>,
    _marker:  PhantomData<T>,
}

impl<'a, T> Ring<'a, T> {
    /// ## Safety
    ///
    /// * ptr must point to a len-sized array of appropriately aligned
    ///   and padded T which should already be initialised.
    ///
    /// Note: will panic if length is 0 or greater than can be
    /// represented in two bits less than half a usize.
    pub unsafe fn from_nonnull_len(ptr: NonNull<MaybeUninit<T>>, len: Half) -> Self {
        assert!(len > 0, "the ring buffer must have a non-zero length");
        assert!(len <= MAX_CAPACITY, "the ring buffer must have a length representable in two bits less than half a usize");
        Self::make(Slice::BorrowedPtrLen(ptr, len))
    }

    /// ## Safety
    ///
    /// * len must not be zero
    /// * ptr must point to a len-sized array of appropriately aligned
    ///   and padded T which should already be initialised.
    ///
    /// Note: will panic if length is 0 or greater than can be
    /// represented in two bits less than half a usize.
    pub unsafe fn from_raw_parts(ptr: *mut MaybeUninit<T>, len: Half) -> Self {
        assert!(len > 0, "the ring buffer must have a non-zero length");
        assert!(len <= MAX_CAPACITY, "the ring buffer must have a length representable in two bits less than half a usize");
        Self::make(Slice::BorrowedPtrLen(NonNull::new_unchecked(ptr), len))
    }

    // the private constructor
    fn make(buffer: Slice<'a, T>) -> Self {
        Ring {
            atomics: Atomics::default(),
            buffer: UnsafeCell::new(buffer),
            _marker: PhantomData,
        }
    }

    // the private destructor
    unsafe fn cleanup(&self, state: State) {
        let _s = self.atomics.sender.take();
        let _r = self.atomics.receiver.take();
        (*self.buffer.get()).cleanup(state);
    }
}

impl<'a, T> From<&'a mut [MaybeUninit<T>]> for Ring<'a, T> {
    fn from(r: &'a mut [MaybeUninit<T>]) -> Self { Self::make(Slice::Ref(r)) }
}
// impl<'a, T> From<Box<[T]>> for Ring<'a, T> {
//     fn from(b: Box<[T]>) -> Self { Self::make(Slice::Box(b)) }
// }
// impl<'a, T> From<Vec<T>> for Ring<'a, T> {
//     fn from(v: Vec<T>) -> Self { Self::make(Slice::Vec(v)) }
// }

#[cfg(feature="alloc")]
struct ContiguousLayout {
    layout: Layout,
    data:   usize,
}
    
#[cfg(feature="alloc")]
impl ContiguousLayout {
    // This only works because of the #[repr(C)] on `Ring`.
    fn for_capacity<T>(size: Half) -> Result<ContiguousLayout, LayoutError> {
        let atomics = Layout::new::<Atomics>();
        let buffer = Layout::new::<Slice<T>>();
        let array = Layout::array::<T>(size as usize)?;
        let (layout, data) = atomics.extend(buffer)?.0.extend(array)?;
        Ok(ContiguousLayout { layout, data })
    }
}

#[derive(Debug,Default)]
struct Atomics {
    state:    AtomicUsize,
    #[cfg(feature="async")]
    sender:   AtomicWaker,
    #[cfg(feature="async")]
    receiver: AtomicWaker,
}
