//! Timeout helpers (linux-gpib style codes and VISA milliseconds).

#[inline]
#[cfg(not(target_os = "windows"))]
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

#[cfg(any(target_os = "windows", test))]
#[inline]
pub(crate) fn secs_to_ms(s: u64) -> u32 {
    if s == 0 {
        // Immediate
        1
    } else {
        s.saturating_mul(1000).min(u64::from(u32::MAX - 1)) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::secs_to_ms;

    #[test]
    fn visa_timeout_conversion_saturates_without_overflowing() {
        assert_eq!(secs_to_ms(0), 1);
        assert_eq!(secs_to_ms(3), 3000);
        assert_eq!(secs_to_ms(u64::MAX), u32::MAX - 1);
    }
}
