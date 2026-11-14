//! Core, backend-agnostic primitives shared by every ChronoFork layer.
//!
//! This crate deliberately contains no emulator-specific types, no transport
//! types and no persistence policy. It defines the vocabulary that sessions,
//! backends and future clients use to talk about identity, emulated time,
//! input and state digests.

#![cfg_attr(test, allow(clippy::expect_used))]

pub mod error;
pub mod hash;
pub mod id;
pub mod input;
pub mod tick;

pub use error::CoreError;
pub use hash::{StateHash, hash_bytes};
pub use id::{SessionId, SnapshotId};
pub use input::InputFrame;
pub use tick::Tick;
