//! Instrument handle and high-throughput helpers.

use std::cmp::min;

#[cfg(not(target_os = "windows"))]
use crate::consts::{EOS_NONE, EOT_ENABLE, ERR, NO_SAD, TIMO, END};
#[cfg(not(target_os = "windows"))]
use crate::ffi::{ibclr, ibdev, ibonl, ibrd, ibtmo, ibwrt};
#[cfg(not(target_os = "windows"))]
use crate::tmo::secs_to_tmo_code;

use crate::error::{Result, check_ok, err, ibsta_val};
use libc::{c_long, c_void};

#[cfg(target_os = "windows")]
use std::ffi::CString;
#[cfg(target_os = "windows")]
use crate::consts::{ERR, TIMO, END};
#[cfg(target_os = "windows")]
use crate::consts::visa::*;
#[cfg(target_os = "windows")]
use crate::ffi::{viOpenDefaultRM, viOpen, viClose, viWrite, viRead, viClear, viSetAttribute, ViSession};
#[cfg(target_os = "windows")]
use crate::error::update_status_from_visa;
#[cfg(target_os = "windows")]
use crate::tmo::secs_to_ms;


/// GPIB instrument handle (closed automatically on Drop).
pub struct Instrument {
    #[cfg(not(target_os = "windows"))]
    pub(crate) ud: i32,
    
    #[cfg(target_os = "windows")]
    pub(crate) rm: ViSession, // We hold our own RM or share? Simplest is local RM.
    #[cfg(target_os = "windows")]
    pub(crate) vi: ViSession,
}

/// Options for `open_with_opts` (clear skipping can speed up).
pub struct OpenOptions {
    pub clear_on_open: bool,
    pub timeout_secs: u64,
}
impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            clear_on_open: false,
            timeout_secs: 10,
        }
    }
}

impl Instrument {
    /// Open on board=0 (gpib0) with default timeout=10s.
    pub fn open(pad: i32) -> Result<Self> {
        Self::open_with(0, pad, 10)
    }

    /// Open with given board index, PAD and timeout seconds.
    pub fn open_with(board: i32, pad: i32, timeout_secs: u64) -> Result<Self> {
        Self::open_with_opts(board, pad, OpenOptions { timeout_secs, clear_on_open: true })
    }

    /// Open with options (optionally skip device clear for speed).
    pub fn open_with_opts(board: i32, pad: i32, opts: OpenOptions) -> Result<Self> {
        #[cfg(target_os = "windows")]
        {
            let mut rm: ViSession = 0;
            let mut vi: ViSession = 0;
            
            unsafe {
                let s1 = viOpenDefaultRM(&mut rm);
                if s1 < VI_SUCCESS {
                    update_status_from_visa(s1, 0);
                    return Err(err("viOpenDefaultRM"));
                }
                
                let rsrc = CString::new(format!("GPIB{}::{}::INSTR", board, pad)).unwrap();
                let s2 = viOpen(rm, rsrc.as_ptr(), 0, 0, &mut vi);
                if s2 < VI_SUCCESS {
                    viClose(rm);
                    update_status_from_visa(s2, 0);
                    return Err(err("viOpen"));
                }
                
                // Set Timeout
                let tmo_ms = secs_to_ms(opts.timeout_secs);
                viSetAttribute(vi, VI_ATTR_TMO_VALUE, tmo_ms as u64);
                // Enable TermChar if needed (usually defaults to LF in many setups or disabled)
                // We typically handle EOS manually or let VISA handle EOI.
                viSetAttribute(vi, VI_ATTR_SEND_END_EN, 1); // Assert EOI at end of write
                
                if opts.clear_on_open {
                    viClear(vi);
                }
                
                update_status_from_visa(VI_SUCCESS, 0);
            }
            
            Ok(Self { rm, vi })
        }

        #[cfg(not(target_os = "windows"))]
        {
            let tmo = secs_to_tmo_code(opts.timeout_secs);
            let ud = unsafe { ibdev(board, pad, NO_SAD, tmo, EOT_ENABLE, EOS_NONE) };
            if ud < 0 {
                return Err(err("ibdev"));
            }
            check_ok("ibdev")?;

            if opts.clear_on_open {
                unsafe {
                    ibclr(ud);
                }
                check_ok("ibclr")?;
            }
            unsafe {
                ibtmo(ud, tmo);
            }
            check_ok("ibtmo")?;
            Ok(Self { ud })
        }
    }

    /// Low-allocation write of a LF-terminated line.
    pub fn write_line_fast(&self, s: &str) -> Result<()> {
        const SBUF: usize = 512;
        // Construct buffer with \n
        let mut stack_buf = [0u8; SBUF];
        let buf_slice: &[u8] = if s.len() < SBUF {
            stack_buf[..s.len()].copy_from_slice(s.as_bytes());
            stack_buf[s.len()] = b'\n';
            &stack_buf[..s.len() + 1]
        } else {
            // fallback to alloc for huge strings
            // (Note: this is slightly different structure than original but cleaner)
            return self.write_line(s);
        };

        self.write_raw(buf_slice)
    }

    /// Read into caller buffer to reduce allocations; returns bytes added.
    pub fn read_into(&self, out: &mut Vec<u8>) -> Result<usize> {
        let mut buf = [0u8; 65536];
        
        #[cfg(target_os = "windows")]
        {
            let mut ret: u32 = 0;
            unsafe {
                let status = viRead(self.vi, buf.as_mut_ptr(), buf.len() as u32, &mut ret);
                update_status_from_visa(status, ret);
            }
            let n = ret as usize;
            if ibsta_val() & TIMO != 0 {
                eprintln!("(warn) read timeout status");
            }
            check_ok("viRead")?;
            out.extend_from_slice(&buf[..n]);
            Ok(n)
        }

        #[cfg(not(target_os = "windows"))]
        {
            unsafe {
                ibrd(
                    self.ud,
                    buf.as_mut_ptr() as *mut c_void,
                    buf.len() as c_long,
                );
            }
            let n = min(super::error::ibcntl_now() as usize, buf.len());
            if (ibsta_val() & TIMO) != 0 {
                eprintln!("(warn) read timeout ibsta=0x{:04x}", ibsta_val());
            }
            check_ok("ibrd")?;
            out.extend_from_slice(&buf[..n]);
            Ok(n)
        }
    }

    /// Query (LF-terminated) into `out`.
    pub fn query_line_into(&self, cmd: &str, out: &mut Vec<u8>) -> Result<()> {
        self.write_line_fast(cmd)?;
        out.clear();
        let _ = self.read_into(out)?;
        
        // Loop until END bit is set (EOI or TermChar)
        // update_status_from_visa sets END properly.
        while (ibsta_val() & END) == 0 {
            let got = self.read_into(out)?;
            if got == 0 {
                break;
            }
        }
        Ok(())
    }

    /// Query IEEE 488.2 binary block; payload goes into `out`.
    pub fn query_ieee_block(&self, cmd: &str, out: &mut Vec<u8>) -> Result<()> {
        self.write_line_fast(cmd)?;
        out.clear();

        let mut head = Vec::with_capacity(32);
        self.read_into(&mut head)?;
        if head.len() < 2 || head[0] != b'#' {
            // Not a binary block or fragmented
            out.extend_from_slice(&head);
            while (ibsta_val() & END) == 0 {
                let _ = self.read_into(out)?;
                if super::error::ibcntl_now() <= 0 {
                    break;
                }
            }
            return Ok(());
        }

        let nd = (head[1] as char).to_digit(10).unwrap_or(0) as usize;
        if nd > 0 {
            while head.len() < 2 + nd {
                let _ = self.read_into(&mut head)?;
            }
            let len = std::str::from_utf8(&head[2..2 + nd])
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);
            out.reserve_exact(len);

            let mut copied = 0usize;
            if head.len() > 2 + nd {
                let take = std::cmp::min(len, head.len() - (2 + nd));
                out.extend_from_slice(&head[2 + nd..2 + nd + take]);
                copied += take;
            }
            while copied < len {
                let before = out.len();
                let _ = self.read_into(out)?;
                let after = out.len();
                if after == before {
                    break;
                }
                copied = after;
            }
            return Ok(());
        }

        // nd == 0 (read until END)
        out.extend_from_slice(&head[2..]);
        while (ibsta_val() & END) == 0 {
            let got = self.read_into(out)?;
            if got == 0 {
                break;
            }
        }
        Ok(())
    }

    // --- Simple, allocation-friendly APIs ---

    pub fn open_default(pad: i32) -> Result<Self> {
        Self::open_with(0, pad, 10)
    }

    pub fn write_line(&self, s: &str) -> Result<()> {
        let mut v = s.as_bytes().to_vec();
        if !v.ends_with(b"\n") {
            v.push(b'\n');
        }
        self.write_raw(&v)
    }

    pub fn write_crlf(&self, s: &str) -> Result<()> {
        let mut v = s.as_bytes().to_vec();
        if !v.ends_with(b"\r\n") {
            v.extend_from_slice(b"\r\n");
        }
        self.write_raw(&v)
    }

    pub fn write_raw(&self, bytes: &[u8]) -> Result<()> {
        #[cfg(target_os = "windows")]
        {
            let mut ret: u32 = 0;
            unsafe {
                let status = viWrite(self.vi, bytes.as_ptr(), bytes.len() as u32, &mut ret);
                update_status_from_visa(status, ret);
            }
            check_ok("viWrite")
        }

        #[cfg(not(target_os = "windows"))]
        {
            unsafe {
                ibwrt(
                    self.ud,
                    bytes.as_ptr() as *const c_void,
                    bytes.len() as c_long,
                );
            }
            check_ok("ibwrt")
        }
    }

    pub fn read_string(&self) -> Result<String> {
        let mut buf = [0u8; 4096];
        
        #[cfg(target_os = "windows")]
        {
            let mut ret: u32 = 0;
            unsafe {
                let status = viRead(self.vi, buf.as_mut_ptr(), buf.len() as u32, &mut ret);
                update_status_from_visa(status, ret);
            }
            let n = ret as usize;
            if ibsta_val() & TIMO != 0 {
                eprintln!("(warn) read timeout");
            }
            check_ok("viRead")?;
            Ok(String::from_utf8_lossy(&buf[..n]).trim_end().to_string())
        }

        #[cfg(not(target_os = "windows"))]
        {
            unsafe {
                ibrd(
                    self.ud,
                    buf.as_mut_ptr() as *mut c_void,
                    buf.len() as c_long,
                );
            }
            let n = min(super::error::ibcntl_now() as usize, buf.len());
            let s = ibsta_val();
            if (s & TIMO) != 0 {
                eprintln!("(warn) read timeout ibsta=0x{s:04x}");
            }
            check_ok("ibrd")?;
            Ok(String::from_utf8_lossy(&buf[..n]).trim_end().to_string())
        }
    }

    pub fn read_all(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            let mut buf = [0u8; 8192];
            let n: usize;
            
            #[cfg(target_os = "windows")]
            {
                let mut ret: u32 = 0;
                unsafe {
                    let status = viRead(self.vi, buf.as_mut_ptr(), buf.len() as u32, &mut ret);
                    update_status_from_visa(status, ret);
                }
                n = ret as usize;
            }

            #[cfg(not(target_os = "windows"))]
            {
                unsafe {
                    ibrd(
                        self.ud,
                        buf.as_mut_ptr() as *mut c_void,
                        buf.len() as c_long,
                    );
                }
                n = min(super::error::ibcntl_now() as usize, buf.len());
            }

            let s = ibsta_val();

            if n > 0 {
                out.extend_from_slice(&buf[..n]);
            }

            if (s & ERR) != 0 {
                if (s & TIMO) != 0 && !out.is_empty() {
                    break;
                }
                return Err(err("read_all"));
            }
            if (s & END) != 0 || n == 0 {
                break;
            }
        }
        Ok(out)
    }

    pub fn query_line(&self, cmd: &str) -> Result<String> {
        self.write_line(cmd)?;
        self.read_string()
    }
    pub fn query_crlf(&self, cmd: &str) -> Result<String> {
        self.write_crlf(cmd)?;
        self.read_string()
    }
    pub fn query_all_line(&self, cmd: &str) -> Result<Vec<u8>> {
        self.write_line(cmd)?;
        self.read_all()
    }

    pub fn set_timeout_secs(&self, secs: u64) -> Result<()> {
        #[cfg(target_os = "windows")]
        {
            let tmo_ms = secs_to_ms(secs);
            unsafe {
                 let s = viSetAttribute(self.vi, VI_ATTR_TMO_VALUE, tmo_ms as u64);
                 update_status_from_visa(s, 0);
            }
            check_ok("viSetAttribute")
        }

        #[cfg(not(target_os = "windows"))]
        {
            let t = secs_to_tmo_code(secs);
            unsafe {
                ibtmo(self.ud, t);
            }
            check_ok("ibtmo")
        }
    }

    pub fn clear(&self) -> Result<()> {
        #[cfg(target_os = "windows")]
        {
            unsafe {
                let s = viClear(self.vi);
                update_status_from_visa(s, 0);
            }
            check_ok("viClear")
        }

        #[cfg(not(target_os = "windows"))]
        {
            unsafe {
                ibclr(self.ud);
            }
            check_ok("ibclr")
        }
    }
}

impl Drop for Instrument {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        unsafe {
            if self.vi != VI_NULL {
                viClose(self.vi);
            }
            if self.rm != VI_NULL {
                viClose(self.rm);
            }
        }

        #[cfg(not(target_os = "windows"))]
        if self.ud >= 0 {
            unsafe {
                ibonl(self.ud, 0);
            }
        }
    }
}