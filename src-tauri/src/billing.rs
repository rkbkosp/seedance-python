use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::models::BalanceSnapshot;

type HmacSha256 = Hmac<Sha256>;

const BILLING_ENDPOINT: &str = "https://open.volcengineapi.com";
const BILLING_HOST: &str = "open.volcengineapi.com";
const BILLING_SERVICE: &str = "billing";
const BILLING_REGION: &str = "cn-beijing";
const BILLING_VERSION: &str = "2022-01-01";
const ACTION_QUERY_BALANCE: &str = "QueryBalanceAcct";

#[derive(Debug, Clone)]
pub struct BillingClient {
    access_key: String,
    secret_key: String,
    security_token: Option<String>,
    client: reqwest::Client,
}

impl BillingClient {
    pub fn new(
        access_key: &str,
        secret_key: &str,
        security_token: Option<String>,
    ) -> Result<Self> {
        if access_key.trim().is_empty() || secret_key.trim().is_empty() {
            bail!("Missing Billing AK/SK");
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        Ok(Self {
            access_key: access_key.trim().to_string(),
            secret_key: secret_key.trim().to_string(),
            security_token,
            client,
        })
    }

    pub async fn query_balance(&self) -> Result<BalanceSnapshot> {
        let now = Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let short_date = now.format("%Y%m%d").to_string();
        let payload = "{}";
        let payload_hash = sha256_hex(payload);
        let signed_headers = if self.security_token.is_some() {
            "content-type;host;x-content-sha256;x-date;x-security-token"
        } else {
            "content-type;host;x-content-sha256;x-date"
        };
        let canonical_headers = format!(
            "content-type:application/json; charset=utf-8\nhost:{BILLING_HOST}\nx-content-sha256:{payload_hash}\nx-date:{amz_date}\n{}",
            self.security_token
                .as_ref()
                .map(|token| format!("x-security-token:{token}\n"))
                .unwrap_or_default()
        );
        let canonical_query = format!("Action={ACTION_QUERY_BALANCE}&Version={BILLING_VERSION}");
        let canonical_request = format!(
            "POST\n/\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );
        let credential_scope = format!("{short_date}/{BILLING_REGION}/{BILLING_SERVICE}/request");
        let string_to_sign = format!(
            "HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
            sha256_hex(&canonical_request)
        );
        let signing_key = signing_key(&self.secret_key, &short_date)?;
        let signature = hex_hmac(&signing_key, &string_to_sign)?;
        let authorization = format!(
            "HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.access_key, credential_scope, signed_headers, signature
        );

        let mut request = self
            .client
            .post(format!("{BILLING_ENDPOINT}/?{canonical_query}"))
            .header("Host", BILLING_HOST)
            .header("Content-Type", "application/json; charset=utf-8")
            .header("X-Date", amz_date)
            .header("X-Content-Sha256", payload_hash)
            .header("Authorization", authorization)
            .body(payload.to_string());
        if let Some(token) = &self.security_token {
            request = request.header("X-Security-Token", token);
        }
        let response = request.send().await?;

        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            bail!("Billing HTTP {status}: {text}");
        }

        let value: Value =
            serde_json::from_str(&text).with_context(|| format!("Invalid Billing JSON: {text}"))?;

        if let Some(error) = value
            .get("ResponseMetadata")
            .and_then(|meta| meta.get("Error"))
        {
            bail!("Billing API error: {}", error);
        }

        let result = value
            .get("Result")
            .ok_or_else(|| anyhow!("Billing response missing Result"))?;

        Ok(BalanceSnapshot {
            account_id: value_to_string(result.get("AccountID")),
            available_balance: value_to_string(result.get("AvailableBalance")),
            cash_balance: value_to_string(result.get("CashBalance")),
            arrears_balance: value_to_string(result.get("ArrearsBalance")),
            credit_limit: value_to_string(result.get("CreditLimit")),
            freeze_amount: value_to_string(result.get("FreezeAmount")),
            updated_at: Some(Utc::now().to_rfc3339()),
            error_message: None,
        })
    }
}

fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex_string(&digest)
}

fn signing_key(secret_key: &str, short_date: &str) -> Result<Vec<u8>> {
    let k_date = hmac_bytes(format!("VOLC{secret_key}").as_bytes(), short_date)?;
    let k_region = hmac_bytes(&k_date, BILLING_REGION)?;
    let k_service = hmac_bytes(&k_region, BILLING_SERVICE)?;
    hmac_bytes(&k_service, "request")
}

fn hmac_bytes(key: &[u8], value: &str) -> Result<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key).context("Failed to create HMAC key")?;
    mac.update(value.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

fn hex_hmac(key: &[u8], value: &str) -> Result<String> {
    Ok(hex_string(&hmac_bytes(key, value)?))
}

fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn value_to_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Number(number)) => Some(number.to_string()),
        Some(other) if !other.is_null() => Some(other.to_string()),
        _ => None,
    }
}
