use sha2::{Digest, Sha256};
use std::fmt::Write;

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    finalize_sha256_hex(Sha256::digest(bytes))
}

pub(crate) fn file_sha256(path: &std::path::Path) -> anyhow::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(finalize_sha256_hex(hasher.finalize()))
}

pub(crate) fn finalize_sha256_hex(digest: impl AsRef<[u8]>) -> String {
    let bytes = digest.as_ref();
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_the_standard_empty_digest() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn finalized_digest_uses_fixed_width_lowercase_hex() {
        assert_eq!(finalize_sha256_hex([0, 1, 15, 16, 255]), "00010f10ff");
    }
}
