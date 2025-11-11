use std::ffi::{CStr, CString};

use crate::conf::{GpibConf, load_gpib_conf, parse_board_index};
use crate::consts::{EOS_NONE, EOT_ENABLE, ERR, NO_SAD};
use crate::driver::ensure_driver_configured;
use crate::error::{Result, check_ok, err, ibsta_val, sys_err};
use crate::ffi::{ibdev, ibfind, ibln, ibonl, ibrsp, ibtmo};
use crate::tmo::secs_to_tmo_code;

/// Represents a GPIB board found via gpib.conf resolution and ibfind.
pub struct Board {
    ud: i32,
    index: i32,
    name: CString,
    controller_pad: i32,
    tmo_code: i32,
    conf: Option<GpibConf>,
}

impl Board {
    /// `request`: "gpib0" / "gpib1" / actual interface name (e.g., "NI GPIB-USB-HS")
    pub fn open(request: &str, timeout_secs: u64) -> Result<Self> {
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

        // eprintln!(
        //     "(info) resolved board: request='{}' -> name='{}', index={}, controller_pad={}",
        //     request, resolved_name, resolved_index, controller_pad
        // );

        let cname = CString::new(resolved_name.clone())
            .map_err(|_| sys_err("CString(name)", "NUL in board name"))?;
        let ud = unsafe { ibfind(cname.as_ptr()) };
        if ud < 0 {
            return Err(err("ibfind(board)"));
        }
        check_ok("ibfind(board)")?;

        let tmo_code = secs_to_tmo_code(timeout_secs);
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

    #[inline]
    pub fn index(&self) -> i32 {
        self.index
    }
    #[inline]
    pub fn name(&self) -> &CStr {
        &self.name
    }

    /// Scan PAD=0..=30; exclude controller PAD unless a device is confirmed there.
    pub fn scan_pads(&self) -> Result<Vec<i32>> {
        let mut v = Vec::new();

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
        if self.ud >= 0 {
            unsafe {
                ibonl(self.ud, 0);
            }
        }
    }
}

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
