use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use hmac::{Hmac, Mac};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::models::BalanceSnapshot;

type HmacSha256 = Hmac<Sha256>;

const BILLING_SERVICE: &str = "billing";
const BILLING_VERSION: &str = "2022-01-01";
const ACTION_QUERY_BALANCE: &str = "QueryBalanceAcct";
const AWS_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

#[derive(Debug, Clone)]
pub struct BillingClient {
    access_key: String,
    secret_key: String,
    client: reqwest::Client,
}

impl BillingClient {
    pub fn new(access_key: &str, secret_key: &str) -> Result<Self> {
        if access_key.trim().is_empty() || secret_key.trim().is_empty() {
            bail!("Missing Billing AK/SK");
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        Ok(Self {
            access_key: access_key.trim().to_string(),
            secret_key: secret_key.trim().to_string(),
            client,
        })
    }

    pub async fn query_balance(&self) -> Result<BalanceSnapshot> {
        let attempts = [
            ("https://billing.volcengineapi.com", "billing.volcengineapi.com", "cn-north-1"),
            ("https://billing.volcengineapi.com", "billing.volcengineapi.com", "cn-beijing"),
            ("https://open.volcengineapi.com", "open.volcengineapi.com", "cn-beijing"),
            ("https://open.volcengineapi.com", "open.volcengineapi.com", "cn-north-1"),
        ];

        let mut last_error: Option<anyhow::Error> = None;
        for (endpoint, host, region) in attempts {
            match self.query_balance_once(endpoint, host, region).await {
                Ok(snapshot) => return Ok(snapshot),
                Err(error) => last_error = Some(error),
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("Failed to query billing balance")))
    }

    async fn query_balance_once(
        &self,
        endpoint: &str,
        host: &str,
        region: &str,
    ) -> Result<BalanceSnapshot> {
        let now = Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let short_date = now.format("%Y%m%d").to_string();
        let canonical_query = canonical_query_string(&[
            ("Action", ACTION_QUERY_BALANCE),
            ("Version", BILLING_VERSION),
        ]);
        let payload_hash = sha256_hex("");
        let signed_headers = "host;x-content-sha256;x-date";
        let canonical_headers = format!(
            "host:{host}\nx-content-sha256:{payload_hash}\nx-date:{amz_date}\n"
        );

        let canonical_request = format!(
            "GET\n/\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );
        let credential_scope = format!("{short_date}/{region}/{BILLING_SERVICE}/request");
        let string_to_sign = format!(
            "HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
            sha256_hex(&canonical_request)
        );
        let signing_key = signing_key(&self.secret_key, &short_date, region)?;
        let signature = hex_hmac(&signing_key, &string_to_sign)?;
        let authorization = format!(
            "HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.access_key, credential_scope, signed_headers, signature
        );

        let url = format!("{endpoint}/?{canonical_query}");
        let response = self
            .client
            .get(url)
            .header("Host", host)
            .header("X-Date", amz_date)
            .header("X-Content-Sha256", payload_hash)
            .header("Authorization", authorization)
            .send()
            .await?;

        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            bail!("Billing HTTP {status} ({host}, {region}): {text}");
        }

        let value: Value =
            serde_json::from_str(&text).with_context(|| format!("Invalid Billing JSON: {text}"))?;

        if let Some(error) = value
            .get("ResponseMetadata")
            .and_then(|meta| meta.get("Error"))
        {
            bail!("Billing API error ({host}, {region}): {}", error);
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

fn canonical_query_string(items: &[(&str, &str)]) -> String {
    let mut pairs = items
        .iter()
        .map(|(key, value)| (encode_component(key), encode_component(value)))
        .collect::<Vec<_>>();
    pairs.sort_unstable();
    pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn encode_component(value: &str) -> String {
    utf8_percent_encode(value, AWS_ENCODE_SET).to_string()
}

fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex_string(&digest)
}

fn signing_key(secret_key: &str, short_date: &str, region: &str) -> Result<Vec<u8>> {
    let k_date = hmac_bytes(format!("VOLC{secret_key}").as_bytes(), short_date)?;
    let k_region = hmac_bytes(&k_date, region)?;
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
