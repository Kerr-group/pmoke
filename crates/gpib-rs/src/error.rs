//! Error type and status helpers.

use crate::consts::ERR;
#[cfg(target_os = "windows")]
use crate::consts::visa::*;
#[cfg(target_os = "windows")]
use crate::consts::{END, TIMO};
use libc::c_long;

#[cfg(not(target_os = "windows"))]
use crate::ffi::{ThreadIbcntl, ThreadIberr, ThreadIbsta};

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

// --- Status Emulation Logic ---

// Thread-local storage to emulate global ibsta/ibcntl on Windows (VISA wrapper)
#[cfg(target_os = "windows")]
use std::cell::RefCell;

#[cfg(target_os = "windows")]
thread_local! {
    pub(crate) static LAST_IBSTA: RefCell<i32> = RefCell::new(0);
    pub(crate) static LAST_IBCNTL: RefCell<c_long> = RefCell::new(0);
    pub(crate) static LAST_IBERR: RefCell<i32> = RefCell::new(0);
}

#[cfg(not(target_os = "windows"))]
#[inline]
pub fn ibsta_now() -> i32 {
    unsafe { ThreadIbsta() }
}
#[cfg(not(target_os = "windows"))]
#[inline]
pub fn iberr_now() -> i32 {
    unsafe { ThreadIberr() }
}
#[cfg(not(target_os = "windows"))]
#[inline]
pub fn ibcntl_now() -> c_long {
    unsafe { ThreadIbcntl() }
}

#[cfg(target_os = "windows")]
#[inline]
pub fn ibsta_now() -> i32 {
    LAST_IBSTA.with(|v| *v.borrow())
}
#[cfg(target_os = "windows")]
#[inline]
pub fn iberr_now() -> i32 {
    LAST_IBERR.with(|v| *v.borrow())
}
#[cfg(target_os = "windows")]
#[inline]
pub fn ibcntl_now() -> c_long {
    LAST_IBCNTL.with(|v| *v.borrow())
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

// --- VISA Status Mapper (Windows) ---

#[cfg(target_os = "windows")]
pub(crate) fn update_status_from_visa(status: i32, cnt: u32) {
    let mut st = 0;

    // Simple mapping: VISA Error < 0 implies failure
    if status < VI_SUCCESS {
        st |= ERR;
        // Map common errors
        if status == VI_ERROR_TMO {
            st |= TIMO;
            set_iberr(6); // EABO
        } else {
            set_iberr(0); // EDVR/Generic
        }
    } else {
        // Success
        set_iberr(0);
        if status == VI_SUCCESS_TERM_CHAR || status == VI_SUCCESS {
            // If VI_SUCCESS_MAX_CNT is NOT set, implies end of transmission usually?
            // Actually, VISA is tricky.
            // VI_SUCCESS_TERM_CHAR -> END
            // VI_SUCCESS -> END (often, if EOI enabled)
            // VI_SUCCESS_MAX_CNT -> Not END (buffer full)
            if status != VI_SUCCESS_MAX_CNT {
                st |= END;
            }
        }
    }

    set_ibsta(st);
    set_ibcntl(cnt as c_long);
}

#[cfg(target_os = "windows")]
fn set_ibsta(v: i32) {
    LAST_IBSTA.with(|c| *c.borrow_mut() = v);
}
#[cfg(target_os = "windows")]
fn set_ibcntl(v: c_long) {
    LAST_IBCNTL.with(|c| *c.borrow_mut() = v);
}
#[cfg(target_os = "windows")]
fn set_iberr(v: i32) {
    LAST_IBERR.with(|c| *c.borrow_mut() = v);
}

