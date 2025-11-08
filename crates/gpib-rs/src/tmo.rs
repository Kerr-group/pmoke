//! Timeout helpers (linux-gpib style codes).

#[inline]
pub(crate) fn secs_to_tmo_code(s: u64) -> i32 {
    match s {
        0..=3 => 11,   // ~3s
        4..=5 => 12,   // ~10s
        6..=10 => 13,  // ~30s
        11..=20 => 14, // ~100s
        21..=30 => 15, // ~300s
        _ => 16,       // ~1000s
    }
}
