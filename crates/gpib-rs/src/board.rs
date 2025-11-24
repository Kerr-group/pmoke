use std::ffi::{CStr, CString};

#[cfg(not(target_os = "windows"))]
use crate::conf::{GpibConf, load_gpib_conf, parse_board_index};
use crate::consts::{EOS_NONE, EOT_ENABLE, ERR, NO_SAD};
#[cfg(not(target_os = "windows"))]
use crate::driver::ensure_driver_configured;
#[cfg(not(target_os = "windows"))]
use crate::error::{ibsta_val, sys_err};
#[cfg(not(target_os = "windows"))]
use crate::ffi::{ibdev, ibfind, ibln, ibonl, ibrsp, ibtmo};
#[cfg(not(target_os = "windows"))]
use crate::tmo::secs_to_tmo_code;

use crate::error::{Result, check_ok, err};

#[cfg(target_os = "windows")]
use crate::ffi::{viOpenDefaultRM, viClose, viFindRsrc, viFindNext, ViSession, ViStatus, ViUInt32};
#[cfg(target_os = "windows")]
use crate::consts::visa::{VI_SUCCESS, VI_NULL};
#[cfg(target_os = "windows")]
use std::os::raw::c_char;

/// Represents a GPIB board.
/// On Linux: Handle to board (ibfind).
/// On Windows: Wrapper around Default Resource Manager (VISA).
pub struct Board {
    #[cfg(not(target_os = "windows"))]
    ud: i32,
    #[cfg(target_os = "windows")]
    rm: ViSession,

    index: i32, // Board index (0 for gpib0)
    
    #[cfg(not(target_os = "windows"))]
    name: CString,
    #[cfg(not(target_os = "windows"))]
    controller_pad: i32,
    #[cfg(not(target_os = "windows"))]
    tmo_code: i32,
    #[cfg(not(target_os = "windows"))]
    conf: Option<GpibConf>,
}

impl Board {
    /// `request`: "gpib0" / "gpib1". On Windows this implies the board number.
    pub fn open(request: &str, _timeout_secs: u64) -> Result<Self> {
        let index = crate::conf::parse_board_index(request).unwrap_or(0);

        #[cfg(target_os = "windows")]
        {
            let mut rm: ViSession = 0;
            unsafe {
                let status = viOpenDefaultRM(&mut rm);
                if status < VI_SUCCESS {
                     return Err(err("viOpenDefaultRM"));
                }
            }
            Ok(Self {
                rm,
                index,
            })
        }

        #[cfg(not(target_os = "windows"))]
        {
            if let Err(e) = ensure_driver_configured() {
                eprintln!("(warn) ensure_driver_configured: {e}");
            }

            let conf_loaded = load_gpib_conf();
            let (resolved_name, resolved_index) =
                resolve_board(request, conf_loaded.as_ref().map(|(c, _)| c));

            let controller_pad = if let Some((conf, _)) = &conf_loaded {
                if let Some(n) = parse_board_index(request) {
                    conf.interfaces
                        .iter()
                        .find(|it| it.minor == n)
                        .and_then(|it| it.pad)
                        .unwrap_or(0)
                } else {
                    conf.interfaces
                        .iter()
                        .find(|it| it.name == resolved_name)
                        .and_then(|it| it.pad)
                        .unwrap_or(0)
                }
            } else {
                0
            };

            let cname = CString::new(resolved_name.clone())
                .map_err(|_| sys_err("CString(name)", "NUL in board name"))?;
            let ud = unsafe { ibfind(cname.as_ptr()) };
            if ud < 0 {
                return Err(err("ibfind(board)"));
            }
            check_ok("ibfind(board)")?;

            let tmo_code = secs_to_tmo_code(_timeout_secs);
            unsafe {
                ibtmo(ud, tmo_code);
            }
            check_ok("ibtmo(board)")?;

            Ok(Self {
                ud,
                index: resolved_index,
                name: cname,
                controller_pad,
                tmo_code,
                conf: conf_loaded.map(|(c, _)| c),
            })
        }
    }

    #[inline]
    pub fn index(&self) -> i32 {
        self.index
    }
    
    #[cfg(not(target_os = "windows"))]
    #[inline]
    pub fn name(&self) -> &CStr {
        &self.name
    }

    /// Scan pads for devices.
    /// Windows: Uses viFindRsrc (fast).
    /// Linux: Uses ibln (listener check) on each PAD.
    pub fn scan_pads(&self) -> Result<Vec<i32>> {
        let mut v = Vec::new();

        #[cfg(target_os = "windows")]
        {
            // Use VISA Resource Manager to find devices instead of brute-force viOpen.
            // Pattern: "GPIB{index}::?*::INSTR" matches all instruments on this board.
            let query = CString::new(format!("GPIB{}::?*::INSTR", self.index)).unwrap();
            
            let mut find_list: ViSession = 0;
            let mut ret_cnt: ViUInt32 = 0;
            let mut desc_buf = [0i8; 256]; // Buffer for resource string (e.g. "GPIB0::17::INSTR")

            unsafe {
                let status = viFindRsrc(
                    self.rm, 
                    query.as_ptr(), 
                    &mut find_list, 
                    &mut ret_cnt, 
                    desc_buf.as_mut_ptr()
                );

                if status >= VI_SUCCESS {
                    // Process the first match
                    if let Some(pad) = parse_visa_rsrc_pad(&desc_buf) {
                         v.push(pad);
                    }

                    // Process subsequent matches
                    while ret_cnt > 1 {
                        let next_status = viFindNext(find_list, desc_buf.as_mut_ptr());
                        if next_status < VI_SUCCESS {
                            break;
                        }
                        if let Some(pad) = parse_visa_rsrc_pad(&desc_buf) {
                             v.push(pad);
                        }
                        // Note: ret_cnt isn't decremented by viFindNext, 
                        // we loop until viFindNext fails or we assume the count was correct.
                        // Ideally checking next_status is sufficient.
                    }
                    viClose(find_list);
                }
            }
            
            // Sort for consistent output
            v.sort_unstable();
            v.dedup();
        }

        #[cfg(not(target_os = "windows"))]
        {
            let device_defined_at_controller = self.conf.as_ref().is_some_and(|conf| {
                conf.devices
                    .iter()
                    .any(|d| d.minor == self.index && d.pad == self.controller_pad)
            });

            for pad in 0..=30 {
                let mut listening: i16 = 0;
                unsafe {
                    ibln(self.ud, pad, NO_SAD, &mut listening as *mut i16);
                }
                if (ibsta_val() & ERR) != 0 || listening == 0 {
                    continue;
                }

                if pad != self.controller_pad {
                    v.push(pad);
                    continue;
                }
                if device_defined_at_controller || self.probe_real_device_on_pad(pad) {
                    v.push(pad);
                }
            }
        }
        Ok(v)
    }

    /// Open an instrument on this board at `pad`.
    pub fn open_instrument(
        &self,
        pad: i32,
        timeout_secs: u64,
    ) -> Result<crate::instrument::Instrument> {
        crate::instrument::Instrument::open_with(self.index, pad, timeout_secs)
    }

    #[cfg(not(target_os = "windows"))]
    fn probe_real_device_on_pad(&self, pad: i32) -> bool {
        let ud = unsafe { ibdev(self.index, pad, NO_SAD, self.tmo_code, EOT_ENABLE, EOS_NONE) };
        if ud < 0 || (ibsta_val() & ERR) != 0 {
            return false;
        }
        let mut spr: i16 = 0;
        unsafe {
            ibrsp(ud, &mut spr as *mut i16);
        }
        let ok = (ibsta_val() & ERR) == 0;
        unsafe {
            ibonl(ud, 0);
        }
        ok
    }
}

impl Drop for Board {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        unsafe {
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

#[cfg(not(target_os = "windows"))]
fn resolve_board(request: &str, conf: Option<&GpibConf>) -> (String, i32) {
    if let Some(n) = crate::conf::parse_board_index(request) {
        if let Some(conf) = conf.and_then(|c| c.interfaces.iter().find(|it| it.minor == n).cloned())
        {
            return (conf.name, conf.minor);
        }
        return (request.to_string(), n);
    }

    if let Some(conf) =
        conf.and_then(|c| c.interfaces.iter().find(|it| it.name == request).cloned())
    {
        return (conf.name, conf.minor);
    }

    (request.to_string(), 0)
}

/// Convenience: scan "gpib0".
pub fn scan_gpib0(timeout_secs: u64) -> Result<Vec<i32>> {
    scan_board("gpib0", timeout_secs)
}
/// Scan a board by "gpibN" or real interface name.
pub fn scan_board(request: &str, timeout_secs: u64) -> Result<Vec<i32>> {
    let b = Board::open(request, timeout_secs)?;
    b.scan_pads()
}

// Helper to parse "GPIB0::17::INSTR" -> 17
#[cfg(target_os = "windows")]
fn parse_visa_rsrc_pad(buf: &[c_char]) -> Option<i32> {
    let s = unsafe { CStr::from_ptr(buf.as_ptr()).to_string_lossy() };
    // Format: "GPIB<board>::<pad>::INSTR" or "GPIB<board>::<pad>::<sad>::INSTR"
    // We want the <pad>.
    let parts: Vec<&str> = s.split("::").collect();
    if parts.len() >= 3 && parts[0].starts_with("GPIB") {
        return parts[1].parse::<i32>().ok();
    }
    None
}