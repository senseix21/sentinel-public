//! Lightweight domain events and bounded pub/sub.
//!
//! Media packets and decoded frames do not belong in this crate.

pub mod bus;
pub mod event;
pub mod publisher;
pub mod subscriber;
