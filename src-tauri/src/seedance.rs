use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use futures_util::StreamExt;
use mime_guess::MimeGuess;
use serde_json::{json, Map, Value};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use url::Url;

use std::path::{Path, PathBuf};

use crate::models::CreateGenerationRequest;

pub const IMAGE_ROLE_FIRST: &str = "first_frame";
pub const IMAGE_ROLE_LAST: &str = "last_frame";
pub const IMAGE_ROLE_REFERENCE: &str = "reference_image";

#[derive(Debug, Clone)]
pub struct ResolvedPlatform {
    pub base_url: String,
    pub model: String,
}

pub fn platform_defaults(platform: &str) -> Result<ResolvedPlatform> {
    match platform {
        "byteplus" => Ok(ResolvedPlatform {
            base_url: "https://ark.ap-southeast.bytepluses.com/api/v3".to_string(),
            model: "seedance-1-5-pro-251215".to_string(),
        }),
        "volc" => Ok(ResolvedPlatform {
            base_url: "https://ark.cn-beijing.volces.com/api/v3".to_string(),
            model: "doubao-seedance-1-5-pro-251215".to_string(),
        }),
        other => bail!("Unsupported platform: {other}"),
    }
}

#[derive(Debug, Clone)]
pub struct SeedanceClient {
    base_url: String,
    client: reqwest::Client,
}

impl SeedanceClient {
    pub fn new(base_url: impl Into<String>, api_key: &str) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {api_key}"))
                .context("Invalid API key for Authorization header")?,
        );
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(300))
            .build()?;

        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            client,
        })
    }

    pub async fn create_task(&self, payload: Value) -> Result<Value> {
        self.request_json(
            self.client
                .post(format!("{}/contents/generations/tasks", self.base_url))
                .json(&payload),
        )
        .await
    }

    pub async fn get_task(&self, task_id: &str) -> Result<Value> {
        self.request_json(
            self.client
                .get(format!("{}/contents/generations/tasks/{task_id}", self.base_url)),
        )
        .await
    }

    pub async fn download_file(&self, url: &str, output: &Path) -> Result<()> {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).await?;
        }

        let response = self.client.get(url).send().await?;
        let status = response.status();
        if !status.is_success() {
            bail!("Failed to download file: HTTP {status}");
        }

        let mut stream = response.bytes_stream();
        let mut file = fs::File::create(output).await?;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(())
    }

    async fn request_json(&self, request: reqwest::RequestBuilder) -> Result<Value> {
        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            bail!("HTTP {status}: {text}");
        }

        let data: Value = serde_json::from_str(&text)
            .with_context(|| format!("Invalid JSON response: {text}"))?;

        if let Some(error) = data.get("error") {
            bail!("{}", error);
        }

        Ok(data)
    }
}

pub async fn build_payload(
    model: &str,
    request: &CreateGenerationRequest,
    first_frame_path: Option<&Path>,
    input_last_frame_path: Option<&Path>,
    reference_paths: &[PathBuf],
) -> Result<Value> {
    let mut payload = Map::new();
    payload.insert("model".to_string(), Value::String(model.to_string()));
    payload.insert(
        "content".to_string(),
        build_content(
            &request.prompt,
            first_frame_path,
            input_last_frame_path,
            reference_paths,
        )
        .await?,
    );

    insert_optional_string(&mut payload, "ratio", request.ratio.as_deref());
    insert_optional_string(&mut payload, "resolution", request.resolution.as_deref());
    insert_optional_i64(&mut payload, "duration", request.duration);
    insert_optional_i64(&mut payload, "frames", request.frames);
    insert_optional_bool(
        &mut payload,
        "return_last_frame",
        request.return_last_frame,
    );
    insert_optional_bool(&mut payload, "draft", request.draft);
    insert_optional_bool(&mut payload, "camera_fixed", request.camera_fixed);
    insert_optional_bool(&mut payload, "watermark", request.watermark);
    insert_optional_bool(&mut payload, "generate_audio", request.generate_audio);
    insert_optional_i64(&mut payload, "seed", request.seed);

    Ok(Value::Object(payload))
}

pub fn extract_status(task: &Value) -> String {
    task.get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

pub fn extract_progress(task: &Value) -> Option<String> {
    match task.get("progress") {
        Some(Value::Number(number)) => Some(format!("{number}%")),
        Some(Value::String(text)) if !text.trim().is_empty() => Some(text.trim().to_string()),
        _ => None,
    }
}

pub fn extract_task_id(task: &Value) -> Result<String> {
    task.get("id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| anyhow!("Task response missing id"))
}

pub fn extract_video_url(task: &Value) -> Option<String> {
    task.get("content")
        .and_then(|content| content.get("video_url"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

pub fn extract_last_frame_url(task: &Value) -> Option<String> {
    task.get("content")
        .and_then(|content| content.get("last_frame_url"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

pub fn extract_error_message(task: &Value) -> Option<String> {
    task.get("error")
        .map(|value| {
            if let Some(text) = value.as_str() {
                text.to_string()
            } else {
                value.to_string()
            }
        })
        .filter(|text| !text.is_empty())
}

pub fn filename_extension_from_url(url: &str, fallback: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|parsed| {
            Path::new(parsed.path())
                .extension()
                .map(|ext| ext.to_string_lossy().to_string())
        })
        .filter(|ext| !ext.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

pub fn output_file_name(generation_id: i64, task_id: Option<&str>, extension: &str) -> String {
    match task_id {
        Some(task_id) if !task_id.is_empty() => format!("{generation_id}-{task_id}.{extension}"),
        _ => format!("{generation_id}.{extension}"),
    }
}

fn insert_optional_string(target: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        target.insert(key.to_string(), Value::String(value.to_string()));
    }
}

fn insert_optional_i64(target: &mut Map<String, Value>, key: &str, value: Option<i64>) {
    if let Some(value) = value {
        target.insert(key.to_string(), json!(value));
    }
}

fn insert_optional_bool(target: &mut Map<String, Value>, key: &str, value: Option<bool>) {
    if let Some(value) = value {
        target.insert(key.to_string(), json!(value));
    }
}

async fn build_content(
    prompt: &str,
    first_frame_path: Option<&Path>,
    input_last_frame_path: Option<&Path>,
    reference_paths: &[PathBuf],
) -> Result<Value> {
    let mut items = vec![json!({ "type": "text", "text": prompt })];

    if let Some(path) = first_frame_path {
        items.push(json!({
            "type": "image_url",
            "image_url": { "url": path_to_data_url(path).await? },
            "role": IMAGE_ROLE_FIRST
        }));
    }

    if let Some(path) = input_last_frame_path {
        items.push(json!({
            "type": "image_url",
            "image_url": { "url": path_to_data_url(path).await? },
            "role": IMAGE_ROLE_LAST
        }));
    }

    for path in reference_paths {
        items.push(json!({
            "type": "image_url",
            "image_url": { "url": path_to_data_url(path).await? },
            "role": IMAGE_ROLE_REFERENCE
        }));
    }

    Ok(Value::Array(items))
}

async fn path_to_data_url(path: &Path) -> Result<String> {
    let bytes = fs::read(path)
        .await
        .with_context(|| format!("Failed to read asset: {}", path.display()))?;
    let mime = MimeGuess::from_path(path)
        .first_or_octet_stream()
        .to_string();
    Ok(format!(
        "data:{mime};base64,{}",
        BASE64_STANDARD.encode(bytes)
    ))
}
