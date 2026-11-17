use chacha20poly1305::{
    aead::{rand_core::RngCore, Aead, KeyInit, OsRng},
    XChaCha20Poly1305, XNonce,
};

pub const KEY_ENV: &str = "FERROGATE_ENCRYPTION_KEY";

const PREFIX: &str = "enc:v1:";
const NONCE_LEN: usize = 24;

#[derive(Debug)]
pub struct KeyError(String);

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KeyError {}

#[derive(Clone)]
pub struct SecretBox {
    cipher: XChaCha20Poly1305,
}

impl std::fmt::Debug for SecretBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretBox")
    }
}

impl SecretBox {
    pub fn from_env() -> Result<Self, KeyError> {
        let raw = std::env::var(KEY_ENV).unwrap_or_default();
        Self::from_encoded(&raw)
    }

    pub fn from_encoded(raw: &str) -> Result<Self, KeyError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(KeyError(format!(
                "{KEY_ENV} is required: it encrypts stored provider credentials. \
                 Generate one with `openssl rand -hex 32`."
            )));
        }

        let bytes = decode_key(trimmed).ok_or_else(|| {
            KeyError(format!(
                "{KEY_ENV} must be 32 bytes encoded as hex (64 chars) or base64"
            ))
        })?;

        Ok(Self {
            cipher: XChaCha20Poly1305::new((&bytes).into()),
        })
    }

    #[doc(hidden)]
    pub fn test_key() -> Self {
        Self::from_encoded("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
            .expect("the built-in test key is valid")
    }

    pub fn is_sealed(value: &str) -> bool {
        value.starts_with(PREFIX)
    }

    pub fn seal(&self, plaintext: &str) -> Result<String, KeyError> {
        if plaintext.is_empty() {
            return Ok(String::new());
        }

        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);

        let ciphertext = self
            .cipher
            .encrypt(&XNonce::from(nonce), plaintext.as_bytes())
            .map_err(|_| KeyError("failed to encrypt credential".to_string()))?;

        let mut packed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        packed.extend_from_slice(&nonce);
        packed.extend_from_slice(&ciphertext);

        Ok(format!("{PREFIX}{}", b64_encode(&packed)))
    }

    pub fn open(&self, stored: &str) -> Result<String, KeyError> {
        if stored.is_empty() || !Self::is_sealed(stored) {
            return Ok(stored.to_string());
        }

        let packed = b64_decode(stored.trim_start_matches(PREFIX))
            .ok_or_else(|| KeyError("stored credential is not valid base64".to_string()))?;

        if packed.len() <= NONCE_LEN {
            return Err(KeyError("stored credential is truncated".to_string()));
        }

        let (nonce, ciphertext) = packed.split_at(NONCE_LEN);
        let plaintext = self
            .cipher
            .decrypt(&XNonce::from(<[u8; NONCE_LEN]>::try_from(nonce).map_err(|_| {
                KeyError("stored credential has a malformed nonce".to_string())
            })?), ciphertext)
            .map_err(|_| {
                KeyError(format!(
                    "a stored credential could not be decrypted; {KEY_ENV} does not match the key it was sealed with"
                ))
            })?;

        String::from_utf8(plaintext)
            .map_err(|_| KeyError("stored credential is not valid UTF-8".to_string()))
    }
}

fn decode_key(raw: &str) -> Option<[u8; 32]> {
    let bytes = match raw.len() {
        64 if raw.chars().all(|c| c.is_ascii_hexdigit()) => hex::decode(raw).ok()?,
        _ => b64_decode(raw)?,
    };

    bytes.try_into().ok()
}

fn b64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;

        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn b64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0;

    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' => continue,
            _ => return None,
        } as u32;

        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    fn boxed() -> SecretBox {
        SecretBox::from_encoded(KEY_HEX).expect("valid key")
    }

    #[test]
    fn round_trips_a_secret() {
        let sealed = boxed().seal("sk-live-abc123").expect("seal");
        assert!(SecretBox::is_sealed(&sealed));
        assert!(
            !sealed.contains("sk-live-abc123"),
            "plaintext leaked: {sealed}"
        );
        assert_eq!(boxed().open(&sealed).expect("open"), "sk-live-abc123");
    }

    #[test]
    fn each_seal_uses_a_fresh_nonce() {
        let s = boxed();
        assert_ne!(s.seal("same").unwrap(), s.seal("same").unwrap());
    }

    #[test]
    fn empty_secret_stays_empty() {
        let s = boxed();
        assert_eq!(s.seal("").unwrap(), "");
        assert_eq!(s.open("").unwrap(), "");
    }

    #[test]
    fn legacy_plaintext_passes_through() {
        assert_eq!(
            boxed().open("sk-old-plaintext").unwrap(),
            "sk-old-plaintext"
        );
    }

    #[test]
    fn a_different_key_cannot_open_it() {
        let sealed = boxed().seal("sk-secret").expect("seal");
        let other = SecretBox::from_encoded(
            "ff0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        )
        .expect("valid key");
        let err = other.open(&sealed).expect_err("must not decrypt");
        assert!(err.to_string().contains(KEY_ENV), "{err}");
    }

    #[test]
    fn a_missing_key_refuses_to_build() {
        let err = SecretBox::from_encoded("   ").expect_err("must refuse");
        assert!(err.to_string().contains(KEY_ENV), "{err}");
    }

    #[test]
    fn a_short_key_is_rejected() {
        assert!(SecretBox::from_encoded("aabbcc").is_err());
    }

    #[test]
    fn debug_never_prints_the_key() {
        assert_eq!(format!("{:?}", boxed()), "SecretBox");
    }

    #[test]
    fn base64_round_trips() {
        for sample in [&b""[..], &b"a"[..], &b"ab"[..], &b"abc"[..], &b"abcd"[..]] {
            assert_eq!(b64_decode(&b64_encode(sample)).unwrap(), sample);
        }
    }
}
