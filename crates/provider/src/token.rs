//! API token generation (DESIGN.md §11.11, Phase 5).
//!
//! Used once, at `monitra start`, when no token was resolved from config or
//! `MONITRA_API_TOKEN`. Hex-encoded by hand rather than pulling in a `hex`
//! crate for something this small.

use rand::RngCore;

/// A fresh 32-byte random token, hex-encoded (64 characters).
pub fn generate_api_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_64_hex_characters() {
        let token = generate_api_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn two_tokens_are_not_equal() {
        assert_ne!(generate_api_token(), generate_api_token());
    }
}
