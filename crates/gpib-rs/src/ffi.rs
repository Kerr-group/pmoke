//! FFI bindings to linux-gpib (Linux/*nix) or VISA (Windows).

use libc::{c_char, c_long, c_void};

#[cfg(target_os = "windows")]
mod windows_defs {
    use super::*;
    // VISA Types
    pub type ViSession = u64; // 64-bit VISA
    pub type ViObject = u64;
    pub type ViStatus = i32;
    pub type ViUInt32 = u32;
    pub type ViUInt16 = u16;
    pub type ViConstString = *const c_char;
    pub type ViAccessMode = u32;
    pub type ViAttr = u32;
    pub type ViBuf = *mut u8;
    pub type ViConstBuf = *const u8;
    pub type ViUInt32Ptr = *mut u32;
    pub type ViAttrState = u64; // Usually logic dependent, u64 covers both 32/64
}

#[cfg(target_os = "windows")]
pub use windows_defs::*;

#[cfg(target_os = "windows")]
#[link(name = "visa64")]
unsafe extern "system" {
    // Resource Manager
    pub fn viOpenDefaultRM(vi: *mut ViSession) -> ViStatus;
    pub fn viFindRsrc(
        sesn: ViSession,
        expr: ViConstString,
        vi: *mut ViSession,
        retCnt: *mut ViUInt32,
        desc: *mut c_char,
    ) -> ViStatus;
    pub fn viFindNext(
        vi: ViSession,
        desc: *mut c_char,
    ) -> ViStatus;
    
    // Resource
    pub fn viOpen(
        sesn: ViSession,
        name: ViConstString,
        mode: ViAccessMode,
        timeout: ViUInt32,
        vi: *mut ViSession,
    ) -> ViStatus;
    pub fn viClose(vi: ViObject) -> ViStatus;
    
    // I/O
    pub fn viWrite(
        vi: ViSession,
        buf: ViConstBuf,
        cnt: ViUInt32,
        retCnt: ViUInt32Ptr,
    ) -> ViStatus;
    pub fn viRead(
        vi: ViSession,
        buf: ViBuf,
        cnt: ViUInt32,
        retCnt: ViUInt32Ptr,
    ) -> ViStatus;
    pub fn viClear(vi: ViSession) -> ViStatus;
    
    // Attributes
    pub fn viSetAttribute(
        vi: ViSession,
        attrName: ViAttr,
        attrValue: ViAttrState,
    ) -> ViStatus;
    pub fn viGetAttribute(
        vi: ViSession,
        attrName: ViAttr,
        attrValue: *mut c_void, 
    ) -> ViStatus;
}

#[cfg(not(target_os = "windows"))]
#[link(name = "gpib")]
unsafe extern "C" {
    pub(crate) fn ThreadIbsta() -> i32;
    pub(crate) fn ThreadIberr() -> i32;
    pub(crate) fn ThreadIbcntl() -> c_long;

    pub(crate) fn ibdev(board: i32, pad: i32, sad: i32, tmo: i32, eot: i32, eos: i32) -> i32;
    pub(crate) fn ibwrt(ud: i32, buf: *const c_void, cnt: c_long) -> i32;
    pub(crate) fn ibrd(ud: i32, buf: *mut c_void, cnt: c_long) -> i32;
    pub(crate) fn ibonl(ud: i32, v: i32) -> i32;
    pub(crate) fn ibclr(ud: i32) -> i32;
    pub(crate) fn ibtmo(ud: i32, tmo: i32) -> i32;
    pub(crate) fn ibln(ud: i32, pad: i32, sad: i32, listen: *mut i16) -> i32;
    pub(crate) fn ibfind(name: *const c_char) -> i32;
    pub(crate) fn ibrsp(ud: i32, spr: *mut i16) -> i32;
}