use async_spsc::*;
use wookie::*;
use core::task::*;

// these helpers make the tests more readable

fn closed<T>(value: T) -> Result<(), SendError<T>> {
    Err(SendError { kind: SendErrorKind::Closed, value})
}
fn full<T>(value: T) -> Result<(), SendError<T>> {
    Err(SendError { kind: SendErrorKind::Full, value})
}

// assert the number of times the waker for the executor has been
// cloned, dropped, woken.
macro_rules! cdw {
    ($pin:ident : $c:literal , $d:literal , $w:literal) => {
        assert_eq!($c, $pin.cloned());
        assert_eq!($d, $pin.dropped());
        assert_eq!($w, $pin.woken());
    }
}

#[test]
fn create_destroy() {
    spsc::<i32>(1);
}

#[test]
fn ping_pong_sync_sync() {
    let (mut s, mut r) = spsc::<i32>(1);
    for _ in 0..10 {
        assert_eq!(Ok(None), r.receive().now());
        assert_eq!(Ok(()), s.send(42).now());
        assert_eq!(full(420), s.send(420).now());
        assert_eq!(Ok(Some(42)), r.receive().now());
        assert_eq!(Ok(None), r.receive().now());
        assert_eq!(Ok(()), s.send(420).now());
        assert_eq!(Ok(Some(420)), r.receive().now());
    }
}

#[test]
fn ping_pong_async_async() {
    unsafe {
        let (mut s, mut r) = spsc::<i32>(1);
        for _ in 0..10 {
            {
                wookie!(r2: r.receive());
                assert_eq!(Poll::Pending, r2.poll());
                r2.stats().assert(1, 0, 0);
                assert_eq!(Poll::Pending, r2.poll());
                r2.stats().assert(2, 1, 0);
                {
                    wookie!(s2: s.send(42));
                    assert_eq!(Poll::Ready(Ok(())), s2.poll());
                    s2.stats().assert(0, 0, 0);
                }
                r2.stats().assert(2, 2, 1);
                wookie!(s2: s.send(420));
                assert_eq!(Poll::Pending, s2.poll());
                r2.stats().assert(2, 2, 1);
                s2.stats().assert(1, 0, 0);
                assert_eq!(Poll::Ready(Ok(42)), r2.poll());
                r2.stats().assert(2, 2, 1);
                s2.stats().assert(1, 1, 1);
                assert_eq!(Poll::Ready(Ok(())), s2.poll());
                s2.stats().assert(1, 1, 1);
            }
            wookie!(r2: r.receive());
            assert_eq!(Poll::Ready(Ok(420)), r2.poll());
            r2.stats().assert(0, 0, 0)
        }
    }
}

#[test]
fn ping_pong_sync_async() {
    unsafe {
        let (mut s, mut r) = spsc::<i32>(1);
        for _ in 0..10 {
            {
                wookie!(r2: r.receive());
                assert_eq!(Poll::Pending, r2.poll());
                r2.stats().assert(1, 0, 0);
                assert_eq!(Poll::Pending, r2.poll());
                r2.stats().assert(2, 1, 0);
                assert_eq!(Ok(()), s.send(42).now());
                r2.stats().assert(2, 2, 1);
                assert_eq!(full(420), s.send(420).now());
                assert_eq!(Poll::Ready(Ok(42)), r2.poll());
            }
            assert_eq!(Ok(()), s.send(420).now());
            wookie!(r2: r.receive());
            assert_eq!(Poll::Ready(Ok(420)), r2.poll());
            r2.stats().assert(0, 0, 0);
        }
    }
}

#[test]
fn ping_pong_async_sync() {
    unsafe {
        let (mut s, mut r) = spsc::<i32>(1);
        for _ in 0..10 {
            assert_eq!(Ok(None), r.receive().now());
            {
                wookie!(s2: s.send(42));
                assert_eq!(Poll::Ready(Ok(())), s2.poll());
                s2.stats().assert(0, 0, 0);
            }
            {
                wookie!(s2: s.send(420));
                assert_eq!(Poll::Pending, s2.poll());
                s2.stats().assert(1, 0, 0);
                assert_eq!(Ok(Some(42)), r.receive().now());
                s2.stats().assert(1, 1, 1);
                assert_eq!(Poll::Ready(Ok(())), s2.poll());
                s2.stats().assert(1, 1, 1);
                assert_eq!(Ok(Some(420)), r.receive().now());
            }
        }
    }
}

#[test]
fn drop_send_now() {
    let (mut s, r) = spsc::<i32>(1);
    drop(r);
    assert_eq!(closed(42), s.send(42).now());
}

#[test]
fn drop_send() {
    unsafe {
        let (mut s, r) = spsc::<i32>(1);
        drop(r);
        wookie!(s2: s.send(42));
        assert_eq!(Poll::Ready(closed(42)), s2.poll());
    }
}

#[test]
fn send_drop() {
    unsafe {
        let (mut s, r) = spsc::<i32>(1);
        {
            wookie!(s2: s.send(42));
            assert_eq!(Poll::Ready(Ok(())), s2.poll());
            cdw!(s2: 0, 0, 0);
        }
        wookie!(s2: s.send(42));
        assert_eq!(Poll::Pending, s2.poll());
        cdw!(s2: 1, 0, 0);
        drop(r);
        cdw!(s2: 1, 1, 1);
        assert_eq!(Poll::Ready(closed(42)), s2.poll());
        cdw!(s2: 1, 1, 1);
    }
}

#[test]
fn drop_receive_now() {
    let (s, mut r) = spsc::<i32>(1);
    drop(s);
    assert_eq!(Err(Closed), r.receive().now());
}

#[test]
fn drop_receive() {
    let (s, mut r) = spsc::<i32>(1);
    drop(s);
    wookie!(r2: r.receive());
    assert_eq!(Poll::Ready(Err(Closed)), r2.poll());
}




#[test]
fn send_drop_receive_now() {
    let (mut s, mut r) = spsc::<i32>(1);
    assert_eq!(Ok(()), s.send(42).now());
    drop(s);
    assert_eq!(Ok(Some(42)), r.receive().now());
}

#[test]
fn send_drop_receive() {
    let (mut s, mut r) = spsc::<i32>(1);
    assert_eq!(Ok(()), s.send(42).now());
    drop(s);
    {
        wookie!(r2: r.receive());
        assert_eq!(Poll::Ready(Ok(42)), r2.poll());
    }
    wookie!(r2: r.receive());
    assert_eq!(Poll::Ready(Err(Closed)), r2.poll());
}
