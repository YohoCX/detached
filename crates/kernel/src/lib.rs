//! The app's live state and its exchange, as GPUI entities on the main thread. It draws nothing:
//! clippy.toml bans GPUI's elements and windows, and `forbid` keeps an `#[allow]` from lifting that.
#![forbid(clippy::disallowed_methods, clippy::disallowed_types)]
