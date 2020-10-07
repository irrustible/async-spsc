use async_spsc::*;
use futures_lite::future::block_on;
use futures_lite::stream::StreamExt;
use futures_micro::prelude::*;

#[test]
fn send_recv() {
    let (mut s, mut r) = bounded::<i32>(1);
    assert_eq!(
        block_on(zip!(async { s.send(42).await.unwrap() }, r.next())),
        ((), Some(42))
    )
}

#[allow(non_snake_case)]
#[test]
fn send__send_recv__recv() {
    let (mut s, mut r) = bounded::<i32>(1);
    assert_eq!(block_on(s.send(42)).unwrap(), ());
    assert_eq!(
        block_on(zip!(async { s.send(43).await.unwrap() }, r.next())),
        ((), Some(42))
    );
    assert_eq!(block_on(r.next()).unwrap(), 43);
}

#[allow(non_snake_case)]
#[test]
fn send__send_recv__close__recv__recv() {
    let (mut s, mut r) = bounded::<i32>(1);
    assert_eq!(block_on(s.send(42)).unwrap(), ());
    assert_eq!(
        block_on(zip!(async { s.send(43).await.unwrap() }, r.next())),
        ((), Some(42))
    );
    s.close();
    assert_eq!(block_on(r.next()).unwrap(), 43);
    assert_eq!(block_on(r.next()), None)
}

#[test]
fn recv_send() {
    let (mut s, mut r) = bounded::<i32>(1);
    assert_eq!(
        block_on(zip!(r.next(), async { s.send(42).await.unwrap() })),
        (Some(42), ())
    )
}

#[allow(non_snake_case)]
#[test]
fn close__recv() {
    let (s, mut r) = bounded::<i32>(1);
    s.close();
    assert_eq!(None, block_on(r.next()));
}

#[allow(non_snake_case)]
#[test]
fn close__send() {
    let (mut s, r) = bounded::<bool>(1);
    r.close();
    assert_eq!(Err(Closed()), block_on(s.send(true)));
}

#[test]
fn send_close() {
    let (mut s, r) = bounded::<bool>(1);
    assert_eq!(
        block_on(zip!(async { s.send(true).await.unwrap() }, async { r.close(); })),
        ((), ())
    );
}

#[allow(non_snake_case)]
#[test]
fn send__send_close() {
    let (mut s, r) = bounded::<bool>(1);
    block_on(s.send(true)).unwrap();
    assert_eq!(
        block_on(zip!(s.send(true), async { r.close(); })),
        (Err(Closed()), ())
    );
}

#[test]
fn recv_close() {
    let (s, mut r) = bounded::<bool>(1);
    assert_eq!(
        block_on(zip!(r.next(), async { s.close() })),
        (None, ())
    )
}

