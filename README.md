# async-spsc

Fast, easy-to-use, async-aware single-producer/single-consumer (SPSC)
channel based around a ringbuffer.

<!-- [![License](https://img.shields.io/crates/l/async-spsc.svg)](https://github.com/irrustible/async-spsc/blob/main/LICENSE) -->
<!-- [![Package](https://img.shields.io/crates/v/async-spsc.svg)](https://crates.io/crates/async-spsc) -->
<!-- [![Documentation](https://docs.rs/async-spsc/badge.svg)](https://docs.rs/async-spsc) -->

## Status: alpha

It works, it's fast, but it's still rough around the edges - missing APIs etc.

We believe it to be correct, but you might not want to deploy to prod just yet.

Git only. The crates.io release is a placeholder.

TODO:

* Batch APIs and streams support.
* More tests.
* More benchmarks.
* More documentation.
* Support use without a global allocator.

Help welcome, I've already spent way more time on this than is healthy...

## Usage

```
use async_spsc::spsc;

async fn async_example() {
  let (mut sender, mut receiver) = spsc::<i32>(2);
  assert!(sender.send(42).await.is_ok());
  assert!(sender.send(420).await.is_ok());
  assert_eq!(receiver.receive().await, Ok(42));
  assert_eq!(receiver.receive().await, Ok(420));
  assert!(sender.send(7).await.is_ok());
  assert_eq!(receiver.receive().await, Ok(7));
}

fn sync_example() {
  let (mut sender, mut receiver) = spsc::<i32>(2);
  assert!(sender.send(42).now().is_ok());
  assert!(sender.send(420).now().is_ok());
  assert!(sender.send(7).now().is_err()); // no space!

  assert_eq!(receiver.receive().now(), Ok(Some(42)));
  assert_eq!(receiver.receive().now(), Ok(Some(420)));
  assert!(receiver.receive().now().is_err()); // no message!

  assert!(sender.send(7).now().is_err());
  assert_eq!(receiver.receive().now(), Ok(Some(7)));
}
```

## Implementation Details

This channel is significantly faster than multi-producer and multi-consumer channels
because it does not require as much synchronisation. It is based around a single
atomic into which we pack two closed flags and two positions. The position is stored
modulo `2*capacity`, but indexed modulo `capacity`. The difference between the two
positions will always be between 0 and `capacity`, both inclusive.

The indices are packed into a single `AtomicUsize`. Each half
additionally reserves a single bit for their `closed` flag. Thus, the
maximum permissible length of a channel must fit into half a usize,
minus two bits:

| usize width (bits) | Maximum length (items) |
|--------------------|------------------------|
|                 64 | 2^30 (over a billion)  |
|                 32 | 2^14 (16384)           |
|                 16 | 2^6  (64)              |

If you try to create a channel longer than this, you will cause a panic.

## Safety

This library consists of low level concurrency and parallelism
primitives. Some things simply could not be done without unsafe,
others would not perform well without it. Thus, we use unsafe code.

We take our use of unsafe seriously. A lot of care and attention has
gone into the design of this library to ensure that we don't invoke
UB. You are encouraged to audit the code and leave thoughtful feedback or
submit improvements.

Broadly speaking, we use unsafe for four main things:

* Pin projection. It isn't optional, we can't use `pin-project-lite`
  at present and we won't use `pin-project`.
* UnsafeCell access. Always appropriately synchronised.
* Dealing with uninitialised memory. The alternative would be to wrap
  each item in an option, but that would consume more memory for most
  message types and be slower.
* Allocating as a single contiguous allocation. This allows us to
  reduce 2 allocations to 1.

## Performance

We're optimised for low contention scenarios - we expect there to be many more
channels than threads on average and there can only be two ends for the channel. If
you are in a potentially high contention scenario such as audio, you might want to
look at one of the (non-async) ring buffers. As always, benchmark real code.

The best performance will be available when we finish the batch APIs (as we can do a
single atomic instead of an atomic per item). We need to figure out the best way of
benchmarking these too.

Here are some unscientific benchmark numbers for a capacity 1 channel. This is
essentially the worst case scenario for this channel because it pays the overheads
of supporting many items, but I can still compare it to
[async-oneshot](https://github.com/irrustible/async-oneshot)'s performance figures
and be disappointed:

```
contiguous_async_1/create_destroy      74.629 ns
contiguous_async_1/send_now_closed     11.625 ns
contiguous_async_1/send_now_empty      15.487 ns
contiguous_async_1/send_now_full       6.8426 ns
contiguous_async_1/receive_now_closed  3.4865 ns
contiguous_async_1/receive_now_empty   5.6765 ns
contiguous_async_1/receive_now_full    14.775 ns
contiguous_async_1/send_closed         11.833 ns
contiguous_async_1/send_empty          18.273 ns
contiguous_async_1/send_full           24.604 ns
contiguous_async_1/receive_closed      6.7487 ns
contiguous_async_1/receive_empty       14.292 ns
contiguous_async_1/receive_full        16.001 ns
```

## Copyright and License

Copyright (c) 2021 James Laver, async-spsc contributors.

[Licensed](LICENSE) under Apache License, Version 2.0 (https://www.apache.org/licenses/LICENSE-2.0),
with LLVM Exceptions (https://spdx.org/licenses/LLVM-exception.html).

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
licensed as above, without any additional terms or conditions.

