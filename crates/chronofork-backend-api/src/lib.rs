//! Contracts between ChronoFork's generic infrastructure and emulation
//! backends.
//!
//! A backend owns emulated hardware state. It does **not** own session
//! identity, forking policy, persistence paths, transports or UI state. Those
//! belong to higher layers and must remain system-independent.

#![cfg_attr(test, allow(clippy::expect_used))]

pub mod descriptor;
pub mod error;
pub mod inspection;
pub mod snapshot;
pub mod traits;

pub use descriptor::{BackendDescriptor, BackendId, Capabilities};
pub use error::BackendError;
pub use inspection::MachineInspection;
pub use snapshot::{BACKEND_SNAPSHOT_FORMAT_VERSION, BackendSnapshot};
pub use traits::{Backend, BackendFactory, StepOutcome};
