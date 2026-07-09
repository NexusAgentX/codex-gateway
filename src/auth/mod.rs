use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::storage::ApiKeyRecord;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AuthenticatedUser {
    pub user_id: String,
    pub api_key_id: String,
    pub key_prefix: String,
    pub email: String,
    pub role: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PanelTokenPayload {
    scope: String,
    user_id: String,
    session_id: String,
    exp: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedApiKey {
    pub plaintext: String,
    pub prefix: String,
    pub hash: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing bearer API key")]
    Missing,
    #[error("invalid API key")]
    Invalid,
    #[error("disabled API key")]
    Disabled,
    #[error("expired API key")]
    Expired,
    #[error("disabled user")]
    DisabledUser,
    #[error(transparent)]
    Storage(#[from] sqlx::Error),
}

pub fn generate_api_key(app_secret: &str) -> PreparedApiKey {
    let mut prefix_bytes = [0_u8; 6];
    let mut secret_bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut prefix_bytes);
    rand::rngs::OsRng.fill_bytes(&mut secret_bytes);

    let prefix = hex::encode(prefix_bytes);
    let secret = URL_SAFE_NO_PAD.encode(secret_bytes);
    let plaintext = format!("cgk_live_{prefix}_{secret}");
    let hash = hash_api_key(app_secret, &plaintext);

    PreparedApiKey {
        plaintext,
        prefix,
        hash,
    }
}

pub fn prepare_existing_api_key(
    app_secret: &str,
    plaintext: &str,
) -> Result<PreparedApiKey, AuthError> {
    let prefix = parse_key_prefix(plaintext).ok_or(AuthError::Invalid)?;
    Ok(PreparedApiKey {
        plaintext: plaintext.to_string(),
        prefix,
        hash: hash_api_key(app_secret, plaintext),
    })
}

pub fn parse_bearer(value: Option<&str>) -> Result<&str, AuthError> {
    let value = value.ok_or(AuthError::Missing)?;
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .ok_or(AuthError::Missing)
}

pub fn parse_key_prefix(key: &str) -> Option<String> {
    let rest = key.strip_prefix("cgk_live_")?;
    let (prefix, secret) = rest.split_once('_')?;
    (!prefix.is_empty() && !secret.is_empty()).then(|| prefix.to_string())
}

pub fn hash_api_key(app_secret: &str, key: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(app_secret.as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(key.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn generate_panel_token(app_secret: &str, user_id: &str) -> String {
    let payload = PanelTokenPayload {
        scope: "panel".to_string(),
        user_id: user_id.to_string(),
        session_id: new_id(),
        exp: (Utc::now() + chrono::Duration::hours(12)).timestamp(),
    };
    sign_panel_payload(app_secret, &payload)
}

pub fn verify_panel_token(app_secret: &str, token: &str) -> Result<(String, String), AuthError> {
    let Some(rest) = token.strip_prefix("cgw_panel_") else {
        return Err(AuthError::Invalid);
    };
    let (payload_b64, signature) = rest.split_once('.').ok_or(AuthError::Invalid)?;
    let expected = panel_signature(app_secret, payload_b64);
    if !verify_hash(&expected, signature) {
        return Err(AuthError::Invalid);
    }
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| AuthError::Invalid)?;
    let payload: PanelTokenPayload =
        serde_json::from_slice(&payload_bytes).map_err(|_| AuthError::Invalid)?;
    if payload.scope != "panel" {
        return Err(AuthError::Invalid);
    }
    if payload.exp < Utc::now().timestamp() {
        return Err(AuthError::Expired);
    }
    Ok((payload.user_id, payload.session_id))
}

pub fn is_panel_token(token: &str) -> bool {
    token.starts_with("cgw_panel_")
}

fn sign_panel_payload(app_secret: &str, payload: &PanelTokenPayload) -> String {
    let payload_json = serde_json::to_vec(payload).expect("panel token payload serializes");
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload_json);
    let signature = panel_signature(app_secret, &payload_b64);
    format!("cgw_panel_{payload_b64}.{signature}")
}

fn panel_signature(app_secret: &str, payload_b64: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(app_secret.as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(b"codex-gateway panel token v1");
    mac.update(payload_b64.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn verify_hash(expected_hash: &str, candidate_hash: &str) -> bool {
    constant_time_eq::constant_time_eq(expected_hash.as_bytes(), candidate_hash.as_bytes())
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|err| anyhow::anyhow!("hashing password: {err}"))?
        .to_string();
    Ok(hash)
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub async fn authenticate_api_key(
    pool: &SqlitePool,
    app_secret: &str,
    authorization: Option<&str>,
) -> Result<AuthenticatedUser, AuthError> {
    let plaintext = parse_bearer(authorization)?;
    let prefix = parse_key_prefix(plaintext).ok_or(AuthError::Invalid)?;
    let candidate_hash = hash_api_key(app_secret, plaintext);
    let record = crate::storage::find_api_key_by_prefix(pool, &prefix)
        .await?
        .ok_or(AuthError::Invalid)?;

    if !verify_hash(&record.key_hash, &candidate_hash) {
        return Err(AuthError::Invalid);
    }
    if record.key_status != "active" {
        return Err(AuthError::Disabled);
    }
    if record.user_status != "active" {
        return Err(AuthError::DisabledUser);
    }
    if is_expired(record.expires_at.as_deref()) {
        return Err(AuthError::Expired);
    }

    crate::storage::mark_api_key_used(pool, &record.api_key_id).await?;

    Ok(AuthenticatedUser {
        user_id: record.user_id,
        api_key_id: record.api_key_id,
        key_prefix: prefix,
        email: record.email,
        role: record.role,
    })
}

fn is_expired(value: Option<&str>) -> bool {
    let Some(value) = value else {
        return false;
    };
    DateTime::parse_from_rfc3339(value)
        .map(|date| date.with_timezone(&Utc) < Utc::now())
        .unwrap_or(false)
}

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn is_admin(user: &AuthenticatedUser) -> bool {
    user.role == "admin"
}

#[allow(dead_code)]
pub fn redact_record(record: &ApiKeyRecord) -> (&str, &str) {
    (&record.api_key_id, &record.key_prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_have_prefix_and_hash() {
        let key = generate_api_key("secret");
        assert!(key.plaintext.starts_with("cgk_live_"));
        assert_eq!(parse_key_prefix(&key.plaintext), Some(key.prefix.clone()));
        assert!(verify_hash(
            &key.hash,
            &hash_api_key("secret", &key.plaintext)
        ));
        assert!(!verify_hash(
            &key.hash,
            &hash_api_key("other", &key.plaintext)
        ));
    }

    #[test]
    fn bearer_parser_rejects_bad_values() {
        assert!(matches!(parse_bearer(None), Err(AuthError::Missing)));
        assert!(matches!(
            parse_bearer(Some("Token abc")),
            Err(AuthError::Missing)
        ));
        assert_eq!(
            parse_bearer(Some("Bearer cgk_live_a_b")).unwrap(),
            "cgk_live_a_b"
        );
    }

    #[test]
    fn password_hash_verifies() {
        let hash = hash_password("correct horse").unwrap();
        assert!(verify_password("correct horse", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn panel_tokens_are_signed_and_scoped() {
        let token = generate_panel_token("secret", "user-1");
        assert!(token.starts_with("cgw_panel_"));
        let (user_id, session_id) = verify_panel_token("secret", &token).unwrap();
        assert_eq!(user_id, "user-1");
        assert!(!session_id.is_empty());
        assert!(verify_panel_token("other-secret", &token).is_err());
    }
}
