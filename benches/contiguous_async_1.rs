#![cfg(all(feature="bench",feature="async"))]
use criterion::*;
use async_spsc::*;
use wookie::dummy;

pub fn create_destroy(c: &mut Criterion) {
    c.bench_function(
        "contiguous_async_1/create_destroy",
        |b| b.iter(|| spsc::<i32>(1))
    );
}

pub fn send_now_closed(c: &mut Criterion) {
    c.bench_function(
        "contiguous_async_1/send_now_closed",
        |b| b.iter_batched_ref(
            || spsc::<i32>(1).0,
            |ref mut s| s.send(420).now().unwrap_err(),
            BatchSize::SmallInput
        )
    );
}

pub fn send_now_empty(c: &mut Criterion) {
    c.bench_function(
        "contiguous_async_1/send_now_empty",
        |b| b.iter_batched_ref(
            || spsc::<i32>(1),
            |(ref mut s, _)| s.send(420).now().unwrap(),
            BatchSize::SmallInput
        )
    );
}

pub fn send_now_full(c: &mut Criterion) {
    c.bench_function(
        "contiguous_async_1/send_now_full",
        |b| b.iter_batched_ref(
            || {
                let (mut s, r) = spsc::<i32>(1);
                s.send(42).now().unwrap();
                (s,r)
            },
            |(ref mut s, _)| s.send(420).now().unwrap_err(),
            BatchSize::SmallInput
        )
    );
}

pub fn receive_now_closed(c: &mut Criterion) {
    c.bench_function(
        "contiguous_async_1/receive_now_closed",
        |b| b.iter_batched_ref(
            || spsc::<i32>(1).1,
            |ref mut r| r.receive().now().unwrap_err(),
            BatchSize::SmallInput
        )
    );
}

pub fn receive_now_empty(c: &mut Criterion) {
    c.bench_function(
        "contiguous_async_1/receive_now_empty",
        |b| b.iter_batched_ref(
            || spsc::<i32>(1),
            |(_, ref mut r)| r.receive().now().unwrap(),
            BatchSize::SmallInput
        )
    );
}

pub fn receive_now_full(c: &mut Criterion) {
    c.bench_function(
        "contiguous_async_1/receive_now_full",
        |b| b.iter_batched_ref(
            || {
                let (mut s, r) = spsc::<i32>(1);
                s.send(420).now().unwrap();
                (s, r)
            },
            |(_, ref mut r)| r.receive().now().unwrap(),
            BatchSize::SmallInput
        )
    );
}

pub fn send_closed(c: &mut Criterion) {
    c.bench_function(
        "contiguous_async_1/send_closed",
        |b| b.iter_batched_ref(
            || spsc::<i32>(1).0,
            |ref mut s| {
                dummy!(s2: s.send(420));
                s2.poll()
            },
            BatchSize::SmallInput
        )
    );
}

pub fn send_empty(c: &mut Criterion) {
    c.bench_function(
        "contiguous_async_1/send_empty",
        |b| b.iter_batched_ref(
            || spsc::<i32>(1),
            |(ref mut s, _)| {
                dummy!(s2: s.send(420));
                s2.poll()
            },
            BatchSize::SmallInput
        )
    );
}

pub fn send_full(c: &mut Criterion) {
    c.bench_function(
        "contiguous_async_1/send_full",
        |b| b.iter_batched_ref(
            || {
                let (mut s, r) = spsc::<i32>(1);
                s.send(42).now().unwrap();
                (s,r)
            },
            |(ref mut s, _)| {
                dummy!(s2: s.send(420));
                s2.poll()
            },
            BatchSize::SmallInput
        )
    );
}

pub fn receive_closed(c: &mut Criterion) {
    c.bench_function(
        "contiguous_async_1/receive_closed",
        |b| b.iter_batched_ref(
            || spsc::<i32>(1).1,
            |ref mut r| {
                dummy!(r2: r.receive());
                r2.poll()
            },
            BatchSize::SmallInput
        )
    );
}

pub fn receive_empty(c: &mut Criterion) {
    c.bench_function(
        "contiguous_async_1/receive_empty",
        |b| b.iter_batched_ref(
            || spsc::<i32>(1),
            |(_, ref mut r)| {
                dummy!(r2: r.receive());
                r2.poll()
            },
            BatchSize::SmallInput
        )
    );
}

pub fn receive_full(c: &mut Criterion) {
    c.bench_function(
        "contiguous_async_1/receive_full",
        |b| b.iter_batched_ref(
            || {
                let (mut s, r) = spsc::<i32>(1);
                s.send(420).now().unwrap();
                (s, r)
            },
            |(_, ref mut r)| {
                dummy!(r2: r.receive());
                r2.poll()
            },
            BatchSize::SmallInput
        )
    );
}




criterion_group!(
    benches,
    create_destroy,
    send_now_closed,
    send_now_empty,
    send_now_full,
    receive_now_closed,
    receive_now_empty,
    receive_now_full,
    send_closed,
    send_empty,
    send_full,
    receive_closed,
    receive_empty,
    receive_full,
);
criterion_main!(benches);
