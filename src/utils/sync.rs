//! Best-effort durability syncs for filesystems without `fsync` support.
//!
//! Rust's `File::sync_all` uses `F_FULLFSYNC` on Apple targets, which some
//! NAS/SMB servers reject with `ENOTSUP`/`EOPNOTSUPP` even though ordinary
//! reads and writes work. Durability syncs only narrow the crash window;
//! they never affect the bytes already written, so on such filesystems the
//! correct behavior is to skip the sync (once warned) rather than abort the
//! run. This is strictly weaker than the lock fallback: skipping a sync can
//! only lose already-written data on a crash, while running without a lock
//! admits concurrent mutation, which stays behind `PMOKE_REQUIRE_LOCK`.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

/// Reports whether an I/O failure means the filesystem does not implement
/// the requested operation at all (NAS/SMB mounts), as opposed to a genuine
/// I/O error that must stay fatal.
pub(crate) fn filesystem_unsupported(error: &io::Error) -> bool {
    match error.raw_os_error() {
        // fcntl locks and F_FULLFSYNC rejected by the filesystem. Linux uses
        // 95 for both ENOTSUP and EOPNOTSUPP; macOS uses 45 for ENOTSUP and
        // 102 for EOPNOTSUPP, so match per OS to avoid misclassifying
        // unrelated errnos (e.g. 102 is ENETRESET on Linux).
        #[cfg(target_os = "macos")]
        Some(45) | Some(102) => true,
        #[cfg(target_os = "linux")]
        Some(95) => true,
        _ => false,
    }
}

static SYNC_UNSUPPORTED_WARNED: AtomicBool = AtomicBool::new(false);

/// Passes through a durability-sync result, swallowing only the
/// filesystem-unsupported case (warned once per process). All other errors,
/// including lock contention surfaced through the same errno space on other
/// platforms, propagate unchanged.
pub(crate) fn tolerate_unsupported_sync(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(error) if filesystem_unsupported(&error) => {
            if !SYNC_UNSUPPORTED_WARNED.swap(true, Ordering::SeqCst) {
                crate::ui::warn(
                    "durability sync (fsync) is not supported on this filesystem (typical on NAS/SMB mounts); continuing without crash-durability guarantees for run artifacts. Prefer a local filesystem for stronger guarantees.",
                );
            }
            Ok(())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_errnos_are_filesystem_signals() {
        assert!(!filesystem_unsupported(&io::Error::new(
            io::ErrorKind::WouldBlock,
            "locked elsewhere"
        )));
        assert!(!filesystem_unsupported(&io::Error::new(
            io::ErrorKind::PermissionDenied,
            "denied"
        )));
        assert!(!filesystem_unsupported(&io::Error::from_raw_os_error(2)));

        #[cfg(target_os = "macos")]
        {
            // ENOTSUP (macOS SMB mounts report this for fcntl locks and
            // F_FULLFSYNC).
            assert!(filesystem_unsupported(&io::Error::from_raw_os_error(45)));
            // EOPNOTSUPP.
            assert!(filesystem_unsupported(&io::Error::from_raw_os_error(102)));
            assert!(!filesystem_unsupported(&io::Error::from_raw_os_error(95)));
        }
        #[cfg(target_os = "linux")]
        {
            // ENOTSUP and EOPNOTSUPP share this value on Linux.
            assert!(filesystem_unsupported(&io::Error::from_raw_os_error(95)));
            assert!(!filesystem_unsupported(&io::Error::from_raw_os_error(45)));
        }
    }

    #[test]
    fn unsupported_sync_is_swallowed_and_others_pass_through() {
        assert!(tolerate_unsupported_sync(Ok(())).is_ok());
        let denied = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        assert!(tolerate_unsupported_sync(Err(denied)).is_err());

        #[cfg(target_os = "macos")]
        for code in [45, 102] {
            let unsupported = io::Error::from_raw_os_error(code);
            assert!(tolerate_unsupported_sync(Err(unsupported)).is_ok());
        }
        #[cfg(target_os = "linux")]
        {
            let unsupported = io::Error::from_raw_os_error(95);
            assert!(tolerate_unsupported_sync(Err(unsupported)).is_ok());
        }
        // Second occurrence stays silent but still succeeds.
        #[cfg(target_os = "macos")]
        {
            let unsupported = io::Error::from_raw_os_error(45);
            assert!(tolerate_unsupported_sync(Err(unsupported)).is_ok());
            let other = io::Error::from_raw_os_error(95);
            assert!(tolerate_unsupported_sync(Err(other)).is_err());
        }
        #[cfg(target_os = "linux")]
        {
            let unsupported = io::Error::from_raw_os_error(95);
            assert!(tolerate_unsupported_sync(Err(unsupported)).is_ok());
            let other = io::Error::from_raw_os_error(45);
            assert!(tolerate_unsupported_sync(Err(other)).is_err());
        }
    }
}
