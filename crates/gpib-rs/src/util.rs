//! Misc helpers (IEEE 488.2 binary block parsing).

/// Extract the payload of an IEEE 488.2 binary block `#<N><len...><payload>`.
pub fn parse_ieee_block(data: &[u8]) -> Option<&[u8]> {
    if data.len() < 2 || data[0] != b'#' {
        return None;
    }
    let nd = (data[1] as char).to_digit(10)? as usize;
    if nd == 0 {
        return Some(&data[2..]);
    }
    if data.len() < 2 + nd {
        return None;
    }
    let len = std::str::from_utf8(&data[2..2 + nd])
        .ok()?
        .parse::<usize>()
        .ok()?;
    let start = 2 + nd;
    if data.len() < start + len {
        return None;
    }
    Some(&data[start..start + len])
}
