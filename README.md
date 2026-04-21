# async-local-channel &emsp; [![Action Badge]][actions] [![Version Badge]][crates.io] [![License Badge]][license] [![Docs Badge]][docs]

[Version Badge]: https://img.shields.io/crates/v/async-local-channel.svg
[crates.io]: https://crates.io/crates/async-local-channel
[Action Badge]: https://github.com/JayceFayne/async-local-channel/workflows/Rust/badge.svg
[actions]: https://github.com/JayceFayne/async-local-channel/actions
[License Badge]: https://img.shields.io/crates/l/async-local-channel.svg
[license]: https://github.com/JayceFayne/async-local-channel/blob/master/LICENSE.md
[Docs Badge]: https://docs.rs/async-local-channel/badge.svg
[docs]: https://docs.rs/async-local-channel

Lightweight channels for single-threaded async runtimes.

## Why?

There are lots of async channel libraries out there. But this one is built specifically for single-threaded async runtimes. All channels are `!Send` and therefore don't use any locks, additionally they are unbounded (with the exception of `oneshot`).

## Contributing

If you find any errors in async-local-channel or just want to add a new feature feel free to [submit a PR](https://github.com/jaycefayne/async-local-channel/pulls).
