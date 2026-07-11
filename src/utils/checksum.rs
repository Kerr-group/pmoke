use sha2::{Digest, Sha256};
use std::fmt::Write;

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    finalize_sha256_hex(Sha256::digest(bytes))
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
