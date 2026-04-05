use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use chrono::Utc;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::pbkdf2;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

use crate::models::AppSettings;

const SECRET_BUNDLE_VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;
const PBKDF2_ITERATIONS: u32 = 120_000;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecretBundleV1 {
    version: u8,
    exported_at: String,
    api_key: String,
    billing_access_key: String,
    billing_secret_key: String,
    billing_security_token: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecretEnvelope {
    version: u8,
    kdf: String,
    iterations: u32,
    algorithm: String,
    salt_b64: String,
    nonce_b64: String,
    ciphertext_b64: String,
}

pub fn export_secret_bundle(settings: &AppSettings, password: &str) -> Result<String> {
    validate_password(password)?;

    let bundle = SecretBundleV1 {
        version: SECRET_BUNDLE_VERSION,
        exported_at: Utc::now().to_rfc3339(),
        api_key: settings.api_key.clone(),
        billing_access_key: settings.billing_access_key.clone(),
        billing_secret_key: settings.billing_secret_key.clone(),
        billing_security_token: settings.billing_security_token.clone(),
    };
    let plaintext = serde_json::to_vec(&bundle)?;

    let rng = SystemRandom::new();
    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rng.fill(&mut salt)
        .map_err(|_| anyhow!("Failed to generate secret bundle salt"))?;
    rng.fill(&mut nonce_bytes)
        .map_err(|_| anyhow!("Failed to generate secret bundle nonce"))?;

    let key = derive_key(password, &salt)?;
    let cipher = LessSafeKey::new(
        UnboundKey::new(&AES_256_GCM, &key)
            .map_err(|_| anyhow!("Failed to initialize encryption key"))?,
    );
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let mut in_out = plaintext;
    cipher
        .seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| anyhow!("Failed to encrypt secret bundle"))?;

    let envelope = SecretEnvelope {
        version: SECRET_BUNDLE_VERSION,
        kdf: "pbkdf2-sha256".to_string(),
        iterations: PBKDF2_ITERATIONS,
        algorithm: "aes-256-gcm".to_string(),
        salt_b64: BASE64_STANDARD.encode(salt),
        nonce_b64: BASE64_STANDARD.encode(nonce_bytes),
        ciphertext_b64: BASE64_STANDARD.encode(in_out),
    };

    let envelope_json = serde_json::to_vec(&envelope)?;
    Ok(BASE64_STANDARD.encode(envelope_json))
}

pub fn import_secret_bundle(current: &AppSettings, password: &str, encoded: &str) -> Result<AppSettings> {
    validate_password(password)?;
    let payload = encoded.trim();
    if payload.is_empty() {
        bail!("Secret bundle is empty");
    }

    let envelope_json = BASE64_STANDARD
        .decode(payload)
        .context("Secret bundle is not valid base64")?;
    let envelope: SecretEnvelope =
        serde_json::from_slice(&envelope_json).context("Secret bundle payload is invalid")?;

    if envelope.version != SECRET_BUNDLE_VERSION {
        bail!("Unsupported secret bundle version: {}", envelope.version);
    }
    if envelope.kdf != "pbkdf2-sha256" || envelope.algorithm != "aes-256-gcm" {
        bail!("Unsupported secret bundle format");
    }

    let salt = BASE64_STANDARD
        .decode(&envelope.salt_b64)
        .context("Secret bundle salt is invalid")?;
    let nonce = BASE64_STANDARD
        .decode(&envelope.nonce_b64)
        .context("Secret bundle nonce is invalid")?;
    let mut ciphertext = BASE64_STANDARD
        .decode(&envelope.ciphertext_b64)
        .context("Secret bundle ciphertext is invalid")?;

    if salt.len() != SALT_LEN || nonce.len() != NONCE_LEN {
        bail!("Secret bundle metadata is malformed");
    }

    let key = derive_key(password, &salt)?;
    let cipher = LessSafeKey::new(
        UnboundKey::new(&AES_256_GCM, &key)
            .map_err(|_| anyhow!("Failed to initialize encryption key"))?,
    );
    let nonce = Nonce::try_assume_unique_for_key(&nonce)
        .map_err(|_| anyhow!("Secret bundle nonce is malformed"))?;
    let plaintext = cipher
        .open_in_place(nonce, Aad::empty(), &mut ciphertext)
        .map_err(|_| anyhow!("Failed to decrypt secret bundle. Check the password."))?;

    let bundle: SecretBundleV1 =
        serde_json::from_slice(plaintext).context("Secret bundle body is invalid")?;
    if bundle.version != SECRET_BUNDLE_VERSION {
        bail!("Unsupported secret payload version: {}", bundle.version);
    }

    let mut next = current.clone();
    next.api_key = bundle.api_key;
    next.billing_access_key = bundle.billing_access_key;
    next.billing_secret_key = bundle.billing_secret_key;
    next.billing_security_token = bundle.billing_security_token;
    Ok(next)
}

fn validate_password(password: &str) -> Result<()> {
    if password.trim().is_empty() {
        bail!("Password is required");
    }
    if password.chars().count() < 8 {
        bail!("Password must be at least 8 characters");
    }
    Ok(())
}

fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; KEY_LEN]> {
    let iterations = NonZeroU32::new(PBKDF2_ITERATIONS)
        .ok_or_else(|| anyhow!("Invalid PBKDF2 iteration count"))?;
    let mut key = [0u8; KEY_LEN];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        iterations,
        salt,
        password.as_bytes(),
        &mut key,
    );
    Ok(key)
}
