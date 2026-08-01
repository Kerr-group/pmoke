//! Prologix GPIB controller client.
//!
//! The crate keeps Prologix transport concerns separate from instrument drivers.
//! Use [`Prologix::write`] for non-query SCPI commands, [`Prologix::query`]
//! for SCPI queries, and [`Prologix::controller_query`] for controller-local
//! diagnostics such as `++ver`.

mod client;
mod config;
mod error;
mod wire;

#[cfg(feature = "serial")]
mod serial;
#[cfg(feature = "tcp")]
mod tcp;

pub use client::Prologix;
pub use config::{
    ControllerConfig, DEFAULT_PORT, DEFAULT_TIMEOUT_MS, MAX_GPIB_ADDRESS, MAX_READ_TIMEOUT_MS,
    MIN_READ_TIMEOUT_MS,
};
pub use error::{Error, Result};

#[cfg(feature = "serial")]
pub use serial::SerialBuilder;
#[cfg(feature = "tcp")]
pub use tcp::TcpBuilder;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
