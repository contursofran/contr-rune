//! hc (herdr-core) client. See `docs/design/HC-PLAN.md` (the Contract is the source of truth).
//!
//! H1: the plain JSON-RPC connection, the dialer (`hc connect` or the in-process fake core) and the
//! watcher that keeps the connection state for Settings.

// Wire types and helpers are used by later parts (H2 and on).
#![allow(dead_code)]

pub mod conn;
pub mod dial;
pub mod fake;
pub mod watch;
pub mod wire;

pub use dial::Dialer;
pub use watch::{HcStatus, HcUpdate};
