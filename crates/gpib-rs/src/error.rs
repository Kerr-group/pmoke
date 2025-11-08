//! Error type and status helpers.

use crate::consts::ERR;
use crate::ffi::{ThreadIbcntl, ThreadIberr, ThreadIbsta};
use libc::c_long;

pub type Result<T> = std::result::Result<T, GpibError>;

#[derive(Debug)]
pub struct GpibError {
    pub ctx: &'static str,
    pub ibsta: i32,
    pub iberr: i32,
    pub note: Option<String>,
}

impl core::fmt::Display for GpibError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Some(note) = &self.note {
            write!(
                f,
                "{} failed: iberr={} [{}], ibsta=0x{:04x} ({})",
                self.ctx,
                self.iberr,
                iberr_name(self.iberr),
                self.ibsta,
                note
            )
        } else {
            write!(
                f,
                "{} failed: iberr={} [{}], ibsta=0x{:04x}",
                self.ctx,
                self.iberr,
                iberr_name(self.iberr),
                self.ibsta
            )
        }
    }
}
impl std::error::Error for GpibError {}

pub(crate) fn iberr_name(e: i32) -> &'static str {
    match e {
        0 => "EDVR (Driver error)",
        1 => "ECIC",
        2 => "ENOL",
        3 => "EADR",
        4 => "EARG",
        5 => "ESAC",
        6 => "EABO (Timeout)",
        7 => "ENEB (No board)",
        10 => "EOIP",
        11 => "ECAP",
        12 => "EFSO",
        13 => "EBUS",
        14 => "ESTB",
        15 => "ESRQ",
        _ => "UNKNOWN",
    }
}

#[inline]
pub fn ibsta_now() -> i32 {
    unsafe { ThreadIbsta() }
}
#[inline]
pub fn iberr_now() -> i32 {
    unsafe { ThreadIberr() }
}
#[inline]
pub fn ibcntl_now() -> c_long {
    unsafe { ThreadIbcntl() }
}

#[inline]
pub(crate) fn ibsta_val() -> i32 {
    ibsta_now()
}
#[inline]
pub(crate) fn iberr_val() -> i32 {
    iberr_now()
}

#[inline(always)]
pub(crate) fn err(ctx: &'static str) -> GpibError {
    GpibError {
        ctx,
        ibsta: ibsta_val(),
        iberr: iberr_val(),
        note: None,
    }
}

#[inline(always)]
pub(crate) fn sys_err(ctx: &'static str, note: impl Into<String>) -> GpibError {
    GpibError {
        ctx,
        ibsta: 0,
        iberr: 0,
        note: Some(note.into()),
    }
}

#[inline(always)]
pub(crate) fn check_ok(ctx: &'static str) -> Result<()> {
    if ibsta_val() & ERR != 0 {
        Err(err(ctx))
    } else {
        Ok(())
    }
}
