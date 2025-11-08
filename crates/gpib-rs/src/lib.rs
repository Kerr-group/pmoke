//! Small, self-contained helpers around linux-gpib / NI-488.2.
//! - Auto-discover gpib.conf (env > common paths > Nix store)
//! - Run `gpib_config -v` if /dev/gpib* is missing (Linux)
//! - Parse gpib.conf to resolve `gpib0` => real board `name`/`minor`/controller PAD
//! - Scan excludes controller PAD unless a real device is confirmed on that PAD
//! - Low-overhead APIs for better throughput

mod board;
mod conf;
mod consts;
mod driver;
mod error;
mod ffi;
mod instrument;
mod tmo;
mod util;

pub use crate::board::{Board, scan_board, scan_gpib0};
pub use crate::error::{GpibError, Result, ibcntl_now, iberr_now, ibsta_now};
pub use crate::instrument::{Instrument, OpenOptions};
pub use crate::util::parse_ieee_block;
