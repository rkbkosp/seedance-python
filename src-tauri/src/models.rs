use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub api_key: String,
    pub platform: String,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub poll_interval: f64,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            platform: "volc".to_string(),
            model: None,
            base_url: None,
            poll_interval: 3.0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInputPayload {
    pub existing_path: Option<String>,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub base64_data: Option<String>,
    pub preview_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGenerationRequest {
    pub prompt: String,
    pub first_frame: Option<FileInputPayload>,
    pub last_frame: Option<FileInputPayload>,
    pub reference_images: Vec<FileInputPayload>,
    pub ratio: Option<String>,
    pub resolution: Option<String>,
    pub duration: Option<i64>,
    pub frames: Option<i64>,
    pub return_last_frame: Option<bool>,
    pub draft: Option<bool>,
    pub camera_fixed: Option<bool>,
    pub watermark: Option<bool>,
    pub generate_audio: Option<bool>,
    pub seed: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationSummary {
    pub id: i64,
    pub task_id: Option<String>,
    pub status: String,
    pub prompt: String,
    pub prompt_summary: String,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub error_message: Option<String>,
    pub progress_text: Option<String>,
    pub video_path: Option<String>,
    pub thumbnail_path: Option<String>,
    pub first_frame_path: Option<String>,
    pub input_last_frame_path: Option<String>,
    pub returned_last_frame_path: Option<String>,
    pub reference_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationDetail {
    pub id: i64,
    pub task_id: Option<String>,
    pub platform: String,
    pub model: String,
    pub status: String,
    pub prompt: String,
    pub prompt_summary: String,
    pub params_json: String,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub error_message: Option<String>,
    pub progress_text: Option<String>,
    pub video_path: Option<String>,
    pub thumbnail_path: Option<String>,
    pub first_frame_path: Option<String>,
    pub input_last_frame_path: Option<String>,
    pub returned_last_frame_path: Option<String>,
    pub reference_count: usize,
    pub reference_images: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub items: Vec<GenerationSummary>,
    pub page: usize,
    pub page_size: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapPayload {
    pub settings: AppSettings,
    pub active_tasks: Vec<GenerationSummary>,
    pub history: HistoryPage,
    pub data_dir: String,
    pub artifacts_dir: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationUpdatedEvent {
    pub generation_id: i64,
}

