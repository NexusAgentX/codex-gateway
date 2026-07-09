use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, KeyInit, OsRng, rand_core::RngCore},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;

const PREFIX: &str = "cgwenc_v1";
const NONCE_LEN: usize = 12;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("invalid encrypted secret format")]
    InvalidFormat,
    #[error("invalid encrypted secret encoding")]
    InvalidEncoding,
    #[error("secret encryption failed")]
    Encrypt,
    #[error("secret decryption failed")]
    Decrypt,
}

pub fn encrypt_upstream_api_key(
    app_secret: &str,
    key_version: i64,
    plaintext: &str,
) -> Result<String, SecretError> {
    let cipher = cipher_for_version(app_secret, key_version);
    let mut nonce = [0_u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes())
        .map_err(|_| SecretError::Encrypt)?;
    Ok(format!(
        "{PREFIX}.{key_version}.{}.{}",
        URL_SAFE_NO_PAD.encode(nonce),
        URL_SAFE_NO_PAD.encode(ciphertext)
    ))
}

pub fn decrypt_upstream_api_key(
    app_secret: &str,
    key_version: i64,
    stored: &str,
) -> Result<String, SecretError> {
    let Some((version, nonce, ciphertext)) = parse_encrypted(stored)? else {
        return Ok(stored.to_string());
    };
    let effective_version = if key_version > 0 {
        key_version
    } else {
        version
    };
    let nonce = URL_SAFE_NO_PAD
        .decode(nonce)
        .map_err(|_| SecretError::InvalidEncoding)?;
    if nonce.len() != NONCE_LEN {
        return Err(SecretError::InvalidEncoding);
    }
    let ciphertext = URL_SAFE_NO_PAD
        .decode(ciphertext)
        .map_err(|_| SecretError::InvalidEncoding)?;
    let plaintext = cipher_for_version(app_secret, effective_version)
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|_| SecretError::Decrypt)?;
    String::from_utf8(plaintext).map_err(|_| SecretError::Decrypt)
}

pub fn is_encrypted_secret(stored: &str) -> bool {
    stored.starts_with(PREFIX)
}

fn parse_encrypted(stored: &str) -> Result<Option<(i64, &str, &str)>, SecretError> {
    if !is_encrypted_secret(stored) {
        return Ok(None);
    }
    let mut parts = stored.split('.');
    let Some(prefix) = parts.next() else {
        return Err(SecretError::InvalidFormat);
    };
    if prefix != PREFIX {
        return Err(SecretError::InvalidFormat);
    }
    let version = parts
        .next()
        .ok_or(SecretError::InvalidFormat)?
        .parse::<i64>()
        .map_err(|_| SecretError::InvalidFormat)?;
    let nonce = parts.next().ok_or(SecretError::InvalidFormat)?;
    let ciphertext = parts.next().ok_or(SecretError::InvalidFormat)?;
    if parts.next().is_some() || version < 1 {
        return Err(SecretError::InvalidFormat);
    }
    Ok(Some((version, nonce, ciphertext)))
}

fn cipher_for_version(app_secret: &str, key_version: i64) -> ChaCha20Poly1305 {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(app_secret.as_bytes())
        .expect("HMAC accepts keys of any length");
    mac.update(b"codex-gateway upstream api key encryption");
    mac.update(&key_version.to_be_bytes());
    let key_bytes = mac.finalize().into_bytes();
    ChaCha20Poly1305::new(Key::from_slice(&key_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypts_and_decrypts_without_plaintext_in_storage_value() {
        let encrypted = encrypt_upstream_api_key("a long test secret", 1, "sk-secret").unwrap();
        assert!(encrypted.starts_with("cgwenc_v1.1."));
        assert!(!encrypted.contains("sk-secret"));
        assert_eq!(
            decrypt_upstream_api_key("a long test secret", 1, &encrypted).unwrap(),
            "sk-secret"
        );
        assert!(decrypt_upstream_api_key("wrong secret", 1, &encrypted).is_err());
    }

    #[test]
    fn legacy_plaintext_rows_are_readable_for_rotation() {
        assert_eq!(
            decrypt_upstream_api_key("secret", 0, "sk-legacy").unwrap(),
            "sk-legacy"
        );
    }
}
