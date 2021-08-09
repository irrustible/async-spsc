use core::ops::{Index, IndexMut};
use core::ptr::{NonNull, drop_in_place};
use crate::*;

#[derive(Debug)]
pub enum Slice<'a, T> {
    Ref(&'a mut [MaybeUninit<T>]),
    BorrowedPtrLen(NonNull<MaybeUninit<T>>, Half),
}

impl<'a, T> Slice<'a, T> {
    #[inline(always)]
    fn capacity(&self) -> Half {
        match self {
            Self::Ref(slice) => slice.len().try_into().unwrap(),
            Self::BorrowedPtrLen(_, len) => *len,
        }
    }
    /// Iterates through the in-flight items, dropping them.
    pub unsafe fn cleanup(&mut self, state: State) {
        let cap = self.capacity();
        let front = state.front().position();
        let mut back = state.back();
        loop {
            let b = back.position();
            if front == b { break; }
            drop_in_place(self.index_mut(b));
            back = back.advance(cap, 1);
        }
    }
}

impl<'a, T> Index<Half> for Slice<'a, T> {
    type Output = MaybeUninit<T>;
    #[inline(always)]
    fn index(&self, index: Half) -> &MaybeUninit<T> {
        let cap = self.capacity();
        let index = (index % cap) as usize;
        match self {
            Self::Ref(slice) => &slice[index],
            Self::BorrowedPtrLen(ptr, _) => unsafe {
                ptr.as_ptr().add(index).as_ref().unwrap()
            }
        }
    }
}

impl<'a, T> IndexMut<Half> for Slice<'a, T> {
    #[inline(always)]
    fn index_mut (&mut self, index: Half) -> &mut MaybeUninit<T> {
        let cap = self.capacity();
        let index = (index % cap) as usize;
        match self {
            Self::Ref(slice) => &mut slice[index],
            Self::BorrowedPtrLen(ptr, _) => unsafe {
                ptr.as_ptr().add(index).as_mut().unwrap()
            }
        }
    }
}
