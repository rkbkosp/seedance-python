mod db;
mod models;
mod seedance;

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use chrono::{Datelike, Utc};
use models::{
    AppSettings, BootstrapPayload, CreateGenerationRequest, FileInputPayload, GenerationDetail,
    GenerationUpdatedEvent, HistoryPage,
};
use rusqlite::Connection;
use seedance::{
    build_payload, extract_error_message, extract_last_frame_url, extract_progress, extract_status,
    extract_task_id, extract_video_url, filename_extension_from_url, output_file_name,
    platform_defaults, ResolvedPlatform, SeedanceClient,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::fs;
use tokio::process::Command;
use tokio::time::{sleep, Duration};

use crate::models::GenerationSummary;

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    data_dir: PathBuf,
    artifacts_dir: PathBuf,
    videos_dir: PathBuf,
    images_dir: PathBuf,
    thumbnails_dir: PathBuf,
}

#[derive(Debug, Clone)]
struct GenerationJob {
    generation_id: i64,
    resolved: ResolvedPlatform,
    request: CreateGenerationRequest,
    first_frame_path: Option<PathBuf>,
    input_last_frame_path: Option<PathBuf>,
    reference_paths: Vec<PathBuf>,
}

#[tauri::command]
fn bootstrap(state: State<'_, AppState>) -> Result<BootstrapPayload, String> {
    let conn = state.db.lock().map_err(lock_error)?;
    let settings = db::load_settings(&conn).map_err(error_to_string)?;
    let active_tasks = db::list_active_generations(&conn).map_err(error_to_string)?;
    let history = db::list_generations(&conn, 1, 10, None).map_err(error_to_string)?;

    Ok(BootstrapPayload {
        settings,
        active_tasks,
        history,
        data_dir: state.data_dir.display().to_string(),
        artifacts_dir: state.artifacts_dir.display().to_string(),
    })
}

#[tauri::command]
fn save_settings(state: State<'_, AppState>, settings: AppSettings) -> Result<AppSettings, String> {
    let sanitized = AppSettings {
        api_key: settings.api_key.trim().to_string(),
        platform: if settings.platform == "byteplus" {
            "byteplus".to_string()
        } else {
            "volc".to_string()
        },
        model: normalize_optional_string(settings.model),
        base_url: normalize_optional_string(settings.base_url),
        poll_interval: if settings.poll_interval > 0.0 {
            settings.poll_interval
        } else {
            3.0
        },
    };

    let conn = state.db.lock().map_err(lock_error)?;
    db::save_settings(&conn, &sanitized).map_err(error_to_string)
}

#[tauri::command]
fn list_generations(
    state: State<'_, AppState>,
    page: Option<usize>,
    page_size: Option<usize>,
    status: Option<String>,
) -> Result<HistoryPage, String> {
    let conn = state.db.lock().map_err(lock_error)?;
    let page = page.unwrap_or(1).max(1);
    let page_size = page_size.unwrap_or(10).clamp(1, 50);
    db::list_generations(&conn, page, page_size, status.as_deref()).map_err(error_to_string)
}

#[tauri::command]
fn get_generation(state: State<'_, AppState>, generation_id: i64) -> Result<GenerationDetail, String> {
    let conn = state.db.lock().map_err(lock_error)?;
    db::get_generation_detail(&conn, generation_id).map_err(error_to_string)
}

#[tauri::command]
fn list_active_generations(state: State<'_, AppState>) -> Result<Vec<GenerationSummary>, String> {
    let conn = state.db.lock().map_err(lock_error)?;
    db::list_active_generations(&conn).map_err(error_to_string)
}

#[tauri::command]
async fn create_generation(
    app: AppHandle,
    state: State<'_, AppState>,
    request: CreateGenerationRequest,
) -> Result<GenerationDetail, String> {
    if request.prompt.trim().is_empty() {
        return Err("Prompt cannot be empty".to_string());
    }
    if request.duration.is_some() && request.frames.is_some() {
        return Err("Use either duration or frames, not both".to_string());
    }

    let settings = {
        let conn = state.db.lock().map_err(lock_error)?;
        db::load_settings(&conn).map_err(error_to_string)?
    };

    if settings.api_key.trim().is_empty() {
        return Err("Missing API key. Save it in Settings first.".to_string());
    }

    let defaults = platform_defaults(&settings.platform).map_err(error_to_string)?;
    let resolved = ResolvedPlatform {
        base_url: settings
            .base_url
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(defaults.base_url),
        model: settings
            .model
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(defaults.model),
    };

    let now = now_string();
    let prompt_summary = summarize_prompt(&request.prompt);
    let params_json = json!({
        "ratio": request.ratio,
        "resolution": request.resolution,
        "duration": request.duration,
        "frames": request.frames,
        "returnLastFrame": request.return_last_frame,
        "draft": request.draft,
        "cameraFixed": request.camera_fixed,
        "watermark": request.watermark,
        "generateAudio": request.generate_audio,
        "seed": request.seed
    })
    .to_string();

    let generation_id = {
        let conn = state.db.lock().map_err(lock_error)?;
        db::insert_generation(
            &conn,
            &settings.platform,
            &resolved.model,
            request.prompt.trim(),
            &prompt_summary,
            &params_json,
            &now,
        )
        .map_err(error_to_string)?
    };

    let first_frame_path = persist_file_input(
        &state,
        generation_id,
        "first-frame",
        request.first_frame.as_ref(),
    )
    .await
    .map_err(error_to_string)?;
    let input_last_frame_path = persist_file_input(
        &state,
        generation_id,
        "last-frame",
        request.last_frame.as_ref(),
    )
    .await
    .map_err(error_to_string)?;

    let mut reference_paths = Vec::new();
    for (index, input) in request.reference_images.iter().enumerate() {
        if let Some(path) = persist_file_input(
            &state,
            generation_id,
            &format!("reference-{}", index + 1),
            Some(input),
        )
        .await
        .map_err(error_to_string)?
        {
            reference_paths.push(path);
        }
    }

    {
        let conn = state.db.lock().map_err(lock_error)?;
        db::update_generation_inputs(
            &conn,
            generation_id,
            first_frame_path.as_deref(),
            input_last_frame_path.as_deref(),
            &reference_paths,
            &now_string(),
        )
        .map_err(error_to_string)?;
    }

    emit_generation_updated(&app, generation_id);

    let job = GenerationJob {
        generation_id,
        resolved,
        request,
        first_frame_path: first_frame_path.map(PathBuf::from),
        input_last_frame_path: input_last_frame_path.map(PathBuf::from),
        reference_paths: reference_paths.iter().map(PathBuf::from).collect(),
    };
    let state_clone = state.inner().clone();
    let app_clone = app.clone();
    let api_key = settings.api_key;
    tauri::async_runtime::spawn(async move {
        run_generation_job(app_clone.clone(), state_clone.clone(), api_key, job).await;
    });

    let conn = state.db.lock().map_err(lock_error)?;
    db::get_generation_detail(&conn, generation_id).map_err(error_to_string)
}

#[tauri::command]
fn open_in_file_manager(path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    if !path.exists() {
        return Err("Path does not exist".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        StdCommand::new("open")
            .arg("-R")
            .arg(path)
            .status()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        StdCommand::new("explorer")
            .arg(format!(r#"/select,{}"#, path.display()))
            .status()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        let target = path.parent().unwrap_or(Path::new("/"));
        StdCommand::new("xdg-open")
            .arg(target)
            .status()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }
}

async fn run_generation_job(
    app: AppHandle,
    state: AppState,
    api_key: String,
    job: GenerationJob,
) {
    let result = async {
        let client = SeedanceClient::new(job.resolved.base_url.clone(), &api_key)?;
        let payload = build_payload(
            &job.resolved.model,
            &job.request,
            job.first_frame_path.as_deref(),
            job.input_last_frame_path.as_deref(),
            &job.reference_paths,
        )
        .await?;

        let create_response = client.create_task(payload).await?;
        let task_id = extract_task_id(&create_response)?;

        {
            let conn = state
                .db
                .lock()
                .map_err(|error| anyhow!(error.to_string()))?;
            db::set_generation_running(
                &conn,
                job.generation_id,
                &task_id,
                Some("starting"),
                &now_string(),
            )?;
        }
        emit_generation_updated(&app, job.generation_id);

        poll_generation_task(app.clone(), state.clone(), client, job.generation_id, task_id).await
    }
    .await;

    if let Err(error) = result {
        finalize_generation_failure(&app, &state, job.generation_id, &error.to_string());
    }
}

async fn poll_generation_task(
    app: AppHandle,
    state: AppState,
    client: SeedanceClient,
    generation_id: i64,
    task_id: String,
) -> Result<()> {
    loop {
        let task = client.get_task(&task_id).await?;
        let status = extract_status(&task);
        let progress = extract_progress(&task);
        let error_message = extract_error_message(&task);

        {
            let conn = state
                .db
                .lock()
                .map_err(|error| anyhow!(error.to_string()))?;
            db::update_generation_progress(
                &conn,
                generation_id,
                &status,
                progress.as_deref(),
                error_message.as_deref(),
                &now_string(),
            )?;
        }
        emit_generation_updated(&app, generation_id);

        if matches!(status.as_str(), "succeeded" | "failed" | "cancelled" | "expired") {
            if status == "succeeded" {
                let video_path = if let Some(video_url) = extract_video_url(&task) {
                    let extension = filename_extension_from_url(&video_url, "mp4");
                    let file_name = output_file_name(generation_id, Some(&task_id), &extension);
                    let target = asset_subpath(&state.videos_dir, &file_name);
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).await?;
                    }
                    client.download_file(&video_url, &target).await?;
                    Some(target)
                } else {
                    None
                };

                let returned_last_frame_path = if let Some(last_frame_url) = extract_last_frame_url(&task) {
                    let extension = filename_extension_from_url(&last_frame_url, "jpg");
                    let file_name = format!("{generation_id}-returned-last-frame.{extension}");
                    let target = asset_subpath(&state.images_dir, &file_name);
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).await?;
                    }
                    client.download_file(&last_frame_url, &target).await?;
                    Some(target)
                } else {
                    None
                };

                let thumbnail_path = create_thumbnail(
                    &state,
                    generation_id,
                    video_path.as_deref(),
                    returned_last_frame_path.as_deref(),
                )
                .await?;

                {
                    let conn = state
                        .db
                        .lock()
                        .map_err(|error| anyhow!(error.to_string()))?;
                    db::finalize_generation(
                        &conn,
                        generation_id,
                        "succeeded",
                        progress.as_deref(),
                        video_path.as_ref().map(path_to_string).as_deref(),
                        thumbnail_path.as_ref().map(path_to_string).as_deref(),
                        returned_last_frame_path.as_ref().map(path_to_string).as_deref(),
                        None,
                        &now_string(),
                    )?;
                }
            } else {
                let conn = state
                    .db
                    .lock()
                    .map_err(|error| anyhow!(error.to_string()))?;
                db::finalize_generation(
                    &conn,
                    generation_id,
                    &status,
                    progress.as_deref(),
                    None,
                    None,
                    None,
                    error_message.as_deref(),
                    &now_string(),
                )?;
            }

            emit_generation_updated(&app, generation_id);
            break;
        }

        let poll_interval = {
            let conn = state
                .db
                .lock()
                .map_err(|error| anyhow!(error.to_string()))?;
            db::load_settings(&conn)?.poll_interval
        };
        let duration = if poll_interval > 0.0 { poll_interval } else { 3.0 };
        sleep(Duration::from_millis((duration * 1000.0) as u64)).await;
    }

    Ok(())
}

fn finalize_generation_failure(app: &AppHandle, state: &AppState, generation_id: i64, message: &str) {
    let timestamp = now_string();
    if let Ok(conn) = state.db.lock() {
        let _ = db::finalize_generation(
            &conn,
            generation_id,
            "failed",
            Some("failed"),
            None,
            None,
            None,
            Some(message),
            &timestamp,
        );
    }
    emit_generation_updated(app, generation_id);
}

fn spawn_resumable_generation_monitor(
    app: AppHandle,
    state: AppState,
    api_key: String,
    generation_id: i64,
    task_id: String,
    resolved: ResolvedPlatform,
) {
    tauri::async_runtime::spawn(async move {
        match SeedanceClient::new(resolved.base_url, &api_key) {
            Ok(client) => {
                if let Err(error) =
                    poll_generation_task(app.clone(), state.clone(), client, generation_id, task_id).await
                {
                    finalize_generation_failure(&app, &state, generation_id, &error.to_string());
                }
            }
            Err(error) => finalize_generation_failure(&app, &state, generation_id, &error.to_string()),
        }
    });
}

fn resume_pending_generations(app: &AppHandle, state: &AppState) -> Result<()> {
    let settings = {
        let conn = state.db.lock().map_err(|error| anyhow!(error.to_string()))?;
        db::load_settings(&conn)?
    };

    if settings.api_key.trim().is_empty() {
        return Ok(());
    }

    let resumable = {
        let conn = state.db.lock().map_err(|error| anyhow!(error.to_string()))?;
        db::list_resumable_generations(&conn)?
    };

    for item in resumable {
        let defaults = platform_defaults(&item.platform)?;
        let resolved = if item.platform == settings.platform {
            ResolvedPlatform {
                base_url: settings
                    .base_url
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(defaults.base_url),
                model: settings
                    .model
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(defaults.model),
            }
        } else {
            defaults
        };

        spawn_resumable_generation_monitor(
            app.clone(),
            state.clone(),
            settings.api_key.clone(),
            item.id,
            item.task_id,
            resolved,
        );
    }

    Ok(())
}

async fn create_thumbnail(
    state: &AppState,
    generation_id: i64,
    video_path: Option<&Path>,
    fallback_image_path: Option<&Path>,
) -> Result<Option<PathBuf>> {
    let target = asset_subpath(&state.thumbnails_dir, &format!("{generation_id}.jpg"));
    if let Some(video_path) = video_path {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).await?;
        }

        let output = Command::new("ffmpeg")
            .arg("-y")
            .arg("-i")
            .arg(video_path)
            .arg("-vf")
            .arg("thumbnail,scale=720:-1")
            .arg("-frames:v")
            .arg("1")
            .arg(&target)
            .output()
            .await;

        if let Ok(output) = output {
            if output.status.success() && target.exists() {
                return Ok(Some(target));
            }
        }
    }

    if let Some(image_path) = fallback_image_path {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::copy(image_path, &target).await?;
        return Ok(Some(target));
    }

    Ok(None)
}

async fn persist_file_input(
    state: &AppState,
    generation_id: i64,
    label: &str,
    input: Option<&FileInputPayload>,
) -> Result<Option<String>> {
    let Some(input) = input else {
        return Ok(None);
    };

    if let Some(existing_path) = input.existing_path.as_ref().filter(|value| !value.trim().is_empty()) {
        let path = PathBuf::from(existing_path);
        if path.exists() {
            return Ok(Some(path_to_string(&path)));
        }
        bail!("Stored asset is missing: {}", path.display());
    }

    let base64_data = input
        .base64_data
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("Missing file content for {label}"))?;
    let bytes = BASE64_STANDARD
        .decode(base64_data)
        .with_context(|| format!("Invalid base64 payload for {label}"))?;

    let extension = detect_extension(input)?;
    let file_name = format!("{generation_id}-{label}.{extension}");
    let target = asset_subpath(&state.images_dir, &file_name);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(&target, bytes).await?;

    Ok(Some(path_to_string(&target)))
}

fn detect_extension(input: &FileInputPayload) -> Result<String> {
    if let Some(file_name) = input.file_name.as_ref() {
        if let Some(extension) = Path::new(file_name).extension() {
            return Ok(extension.to_string_lossy().to_string());
        }
    }

    if let Some(mime_type) = input.mime_type.as_ref() {
        if let Some(extensions) = mime_guess::get_mime_extensions_str(mime_type) {
            if let Some(extension) = extensions.first() {
                return Ok((*extension).to_string());
            }
        }
    }

    Ok("bin".to_string())
}

fn asset_subpath(root: &Path, file_name: &str) -> PathBuf {
    let now = Utc::now();
    root.join(format!("{:04}", now.year()))
        .join(format!("{:02}", now.month()))
        .join(file_name)
}

fn summarize_prompt(prompt: &str) -> String {
    let collapsed = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = collapsed.chars();
    let summary: String = chars.by_ref().take(120).collect();
    if chars.next().is_some() {
        format!("{summary}...")
    } else {
        summary
    }
}

fn now_string() -> String {
    Utc::now().to_rfc3339()
}

fn path_to_string<P: AsRef<Path>>(path: P) -> String {
    path.as_ref().display().to_string()
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|item| {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn emit_generation_updated(app: &AppHandle, generation_id: i64) {
    let _ = app.emit(
        "generation-updated",
        GenerationUpdatedEvent { generation_id },
    );
}

fn error_to_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn lock_error<T>(error: std::sync::PoisonError<T>) -> String {
    error.to_string()
}

fn build_app_state(app: &tauri::App) -> Result<AppState> {
    let data_dir = app
        .path()
        .app_data_dir()
        .context("Failed to resolve app data dir")?;
    let artifacts_dir = data_dir.join("artifacts");
    let videos_dir = artifacts_dir.join("videos");
    let images_dir = artifacts_dir.join("images");
    let thumbnails_dir = artifacts_dir.join("thumbnails");

    std::fs::create_dir_all(&videos_dir)?;
    std::fs::create_dir_all(&images_dir)?;
    std::fs::create_dir_all(&thumbnails_dir)?;

    let db_path = data_dir.join("seedance.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(db_path)?;
    db::init_schema(&conn)?;

    Ok(AppState {
        db: Arc::new(Mutex::new(conn)),
        data_dir,
        artifacts_dir,
        videos_dir,
        images_dir,
        thumbnails_dir,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let state = build_app_state(app)?;
            app.manage(state.clone());
            resume_pending_generations(app.handle(), &state)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            save_settings,
            list_generations,
            get_generation,
            list_active_generations,
            create_generation,
            open_in_file_manager
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
