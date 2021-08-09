use core::convert::TryInto;

#[cfg(target_pointer_width="64")] pub type Half = u32;
#[cfg(target_pointer_width="32")] pub type Half = u16;
#[cfg(target_pointer_width="16")] pub type Half = u8;

#[cfg(target_pointer_width="64")] pub const BITS: usize = 32;
#[cfg(target_pointer_width="32")] pub const BITS: usize = 16;
#[cfg(target_pointer_width="16")] pub const BITS: usize = 8;

pub const HIGH_BIT:     Half = 1 << (BITS - 1);
pub const S_CLOSE:      usize = HIGH_BIT as usize;
pub const R_CLOSE:      usize = S_CLOSE << BITS;
pub const ANY_CLOSE:    usize = S_CLOSE | R_CLOSE;
pub const FRONT:        usize = Half::MAX as usize;
pub const BACK:         usize = !FRONT;
pub const MAX_CAPACITY: Half  = (HIGH_BIT >> 1) - 1;

#[derive(Copy,Clone,Debug,Eq,PartialEq)]
pub struct HalfState(pub Half);

impl HalfState {

    #[inline(always)]
    pub fn position(self) -> Half { self.0 & !(1 << (BITS - 1)) }

    #[inline(always)]
    pub fn is_closed(self) -> bool { (self.0 & HIGH_BIT) != 0 }

    #[inline(always)]
    pub fn advance(self, cap: Half, by: Half) -> Self {
        HalfState((self.0 + by) % (2 * cap))
    }

    #[inline(always)]
    pub fn close(self) -> Self { HalfState(self.0 | HIGH_BIT) }
}

/// The state is divided into two halves: front (updated by Sender)
/// and back (updated by Receiver). Each half comprises an integer
/// index into the slice and a 'closed' flag.
///
/// Ring buffers have a small complication in general: how to
/// distinguish full and empty. There are a few approaches the
/// internet knows about:
///
/// 1. Wrap at capacity when incrementing and waste a slot. Naive,
///    simple, no thanks.
/// 2. Don't wrap when incrementing and wrap at capacity when
///    indexing.  This allows use of the full integer to represent how
///    far ahead of the back the front is, though we only need one
///    extra value. A variation of this does not do overflow checking
///    but requires the buffer size to be a power of two.
/// 3. Wrap at twice capacity when incrementing and at capacity when
///    indexing. This allows us to get away with only stealing one
///    bit. This is very handy for us since we're trying to use as
///    little space as possible so we can pack them into a single
///    atomic. This is the approach we chose.
#[derive(Copy,Clone,Debug,Eq,PartialEq)]
pub struct State(pub(crate) usize);

impl State {
    #[inline(always)]
    pub fn front(self) -> HalfState {
        HalfState((self.0 & FRONT).try_into().unwrap())
    }

    #[inline(always)]
    pub fn back(self) -> HalfState {
        HalfState((self.0 >> BITS).try_into().unwrap())
    }

    #[inline(always)]
    pub fn with_front(self, front: HalfState) -> State {
        State((self.0 & BACK) | front.0 as usize)
    }

    #[inline(always)]
    pub fn with_back(self, back: HalfState) -> State {
        State((self.0 & FRONT) | ((back.0 as usize) << BITS) )
    }

    #[inline(always)]
    pub fn is_closed(self) -> bool { (self.0 & ANY_CLOSE) != 0 }

    #[inline(always)]
    pub fn is_full(self, cap: Half) -> bool { self.len(cap) == cap }

    #[inline(always)]
    pub fn is_empty(self) -> bool { self.front().position() == self.back().position() }

    /// The number of slots available for writing.
    #[inline(always)]
    pub fn space(self, cap: Half) -> Half { cap - self.len(cap)}
    /// The number of slots available for reading.

    #[inline(always)]
    pub fn len(self, cap: Half) -> Half {
        let f = self.front().position();
        let b = self.back().position();
        if f >= b {
            f - b
        } else {
            2 * cap - b + f
        }
    }
}
