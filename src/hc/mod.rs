//! hc (herdr-core) client. See `docs/design/HC-PLAN.md` (the Contract is the source of truth).
//!
//! H1: the plain JSON-RPC connection, the dialer (`hc connect` or the in-process fake core) and the
//! watcher that keeps the connection state for Settings. H2: the framed codec and connection
//! (`framed`) and the fake core's framed mode.

pub mod conn;
pub mod dial;
pub mod fake;
pub mod framed;
pub mod model;
pub mod owner;
pub mod restore;
pub mod term;
pub mod watch;
pub mod wire;

pub use dial::Dialer;
pub use watch::{HcStatus, HcUpdate};
