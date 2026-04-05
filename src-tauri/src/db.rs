use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::models::{AppSettings, GenerationDetail, GenerationSummary, HistoryPage};

pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS app_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            api_key TEXT NOT NULL DEFAULT '',
            platform TEXT NOT NULL DEFAULT 'volc',
            model TEXT,
            base_url TEXT,
            poll_interval REAL NOT NULL DEFAULT 3.0
        );

        INSERT OR IGNORE INTO app_settings (id, api_key, platform, poll_interval)
        VALUES (1, '', 'volc', 3.0);

        CREATE TABLE IF NOT EXISTS generations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id TEXT,
            platform TEXT NOT NULL,
            model TEXT NOT NULL,
            status TEXT NOT NULL,
            prompt TEXT NOT NULL,
            prompt_summary TEXT NOT NULL,
            params_json TEXT NOT NULL,
            progress_text TEXT,
            video_path TEXT,
            thumbnail_path TEXT,
            first_frame_path TEXT,
            input_last_frame_path TEXT,
            returned_last_frame_path TEXT,
            error_message TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            completed_at TEXT
        );

        CREATE TABLE IF NOT EXISTS reference_images (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            generation_id INTEGER NOT NULL,
            image_path TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            sort_order INTEGER NOT NULL,
            FOREIGN KEY (generation_id) REFERENCES generations(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_generations_created_at ON generations(created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_generations_status ON generations(status);
        CREATE INDEX IF NOT EXISTS idx_reference_images_generation_id ON reference_images(generation_id);
        "#,
    )?;

    Ok(())
}

pub fn load_settings(conn: &Connection) -> Result<AppSettings> {
    let settings = conn
        .query_row(
            r#"
            SELECT api_key, platform, model, base_url, poll_interval
            FROM app_settings
            WHERE id = 1
            "#,
            [],
            |row| {
                Ok(AppSettings {
                    api_key: row.get(0)?,
                    platform: row.get(1)?,
                    model: row.get(2)?,
                    base_url: row.get(3)?,
                    poll_interval: row.get(4)?,
                })
            },
        )
        .optional()?;

    Ok(settings.unwrap_or_default())
}

pub fn save_settings(conn: &Connection, settings: &AppSettings) -> Result<AppSettings> {
    conn.execute(
        r#"
        INSERT INTO app_settings (id, api_key, platform, model, base_url, poll_interval)
        VALUES (1, ?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(id) DO UPDATE SET
            api_key = excluded.api_key,
            platform = excluded.platform,
            model = excluded.model,
            base_url = excluded.base_url,
            poll_interval = excluded.poll_interval
        "#,
        params![
            settings.api_key,
            settings.platform,
            settings.model,
            settings.base_url,
            settings.poll_interval
        ],
    )?;

    load_settings(conn)
}

pub fn insert_generation(
    conn: &Connection,
    platform: &str,
    model: &str,
    prompt: &str,
    prompt_summary: &str,
    params_json: &str,
    now: &str,
) -> Result<i64> {
    conn.execute(
        r#"
        INSERT INTO generations (
            platform,
            model,
            status,
            prompt,
            prompt_summary,
            params_json,
            created_at,
            updated_at
        )
        VALUES (?1, ?2, 'queued', ?3, ?4, ?5, ?6, ?6)
        "#,
        params![platform, model, prompt, prompt_summary, params_json, now],
    )?;

    Ok(conn.last_insert_rowid())
}

pub fn update_generation_inputs(
    conn: &Connection,
    generation_id: i64,
    first_frame_path: Option<&str>,
    input_last_frame_path: Option<&str>,
    reference_images: &[String],
    now: &str,
) -> Result<()> {
    conn.execute(
        r#"
        UPDATE generations
        SET first_frame_path = ?2,
            input_last_frame_path = ?3,
            updated_at = ?4
        WHERE id = ?1
        "#,
        params![generation_id, first_frame_path, input_last_frame_path, now],
    )?;

    conn.execute(
        "DELETE FROM reference_images WHERE generation_id = ?1",
        params![generation_id],
    )?;

    for (index, path) in reference_images.iter().enumerate() {
        conn.execute(
            r#"
            INSERT INTO reference_images (generation_id, image_path, source_kind, sort_order)
            VALUES (?1, ?2, 'uploaded', ?3)
            "#,
            params![generation_id, path, index as i64],
        )?;
    }

    Ok(())
}

pub fn set_generation_running(
    conn: &Connection,
    generation_id: i64,
    task_id: &str,
    progress_text: Option<&str>,
    now: &str,
) -> Result<()> {
    conn.execute(
        r#"
        UPDATE generations
        SET task_id = ?2,
            status = 'running',
            progress_text = ?3,
            updated_at = ?4
        WHERE id = ?1
        "#,
        params![generation_id, task_id, progress_text, now],
    )?;
    Ok(())
}

pub fn update_generation_progress(
    conn: &Connection,
    generation_id: i64,
    status: &str,
    progress_text: Option<&str>,
    error_message: Option<&str>,
    now: &str,
) -> Result<()> {
    conn.execute(
        r#"
        UPDATE generations
        SET status = ?2,
            progress_text = ?3,
            error_message = ?4,
            updated_at = ?5
        WHERE id = ?1
        "#,
        params![generation_id, status, progress_text, error_message, now],
    )?;
    Ok(())
}

pub fn finalize_generation(
    conn: &Connection,
    generation_id: i64,
    status: &str,
    progress_text: Option<&str>,
    video_path: Option<&str>,
    thumbnail_path: Option<&str>,
    returned_last_frame_path: Option<&str>,
    error_message: Option<&str>,
    now: &str,
) -> Result<()> {
    conn.execute(
        r#"
        UPDATE generations
        SET status = ?2,
            progress_text = ?3,
            video_path = ?4,
            thumbnail_path = ?5,
            returned_last_frame_path = ?6,
            error_message = ?7,
            updated_at = ?8,
            completed_at = CASE
                WHEN ?2 IN ('succeeded', 'failed', 'cancelled', 'expired') THEN ?8
                ELSE completed_at
            END
        WHERE id = ?1
        "#,
        params![
            generation_id,
            status,
            progress_text,
            video_path,
            thumbnail_path,
            returned_last_frame_path,
            error_message,
            now
        ],
    )?;
    Ok(())
}

pub fn list_generations(
    conn: &Connection,
    page: usize,
    page_size: usize,
    status_filter: Option<&str>,
) -> Result<HistoryPage> {
    let offset = page.saturating_sub(1) * page_size;
    let total = count_generations(conn, status_filter)?;
    let items = if let Some(status) = status_filter {
        let mut stmt = conn.prepare(
            r#"
            SELECT
                g.id,
                g.task_id,
                g.status,
                g.prompt,
                g.prompt_summary,
                g.created_at,
                g.updated_at,
                g.completed_at,
                g.error_message,
                g.progress_text,
                g.video_path,
                g.thumbnail_path,
                g.first_frame_path,
                g.input_last_frame_path,
                g.returned_last_frame_path,
                (SELECT COUNT(*) FROM reference_images r WHERE r.generation_id = g.id) AS reference_count
            FROM generations g
            WHERE g.status = ?1
            ORDER BY g.created_at DESC
            LIMIT ?2 OFFSET ?3
            "#,
        )?;
        let rows = stmt
            .query_map(params![status, page_size as i64, offset as i64], row_to_summary)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    } else {
        let mut stmt = conn.prepare(
            r#"
            SELECT
                g.id,
                g.task_id,
                g.status,
                g.prompt,
                g.prompt_summary,
                g.created_at,
                g.updated_at,
                g.completed_at,
                g.error_message,
                g.progress_text,
                g.video_path,
                g.thumbnail_path,
                g.first_frame_path,
                g.input_last_frame_path,
                g.returned_last_frame_path,
                (SELECT COUNT(*) FROM reference_images r WHERE r.generation_id = g.id) AS reference_count
            FROM generations g
            ORDER BY g.created_at DESC
            LIMIT ?1 OFFSET ?2
            "#,
        )?;
        let rows = stmt
            .query_map(params![page_size as i64, offset as i64], row_to_summary)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    Ok(HistoryPage {
        items,
        page,
        page_size,
        total,
    })
}

pub fn list_active_generations(conn: &Connection) -> Result<Vec<GenerationSummary>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT
            g.id,
            g.task_id,
            g.status,
            g.prompt,
            g.prompt_summary,
            g.created_at,
            g.updated_at,
            g.completed_at,
            g.error_message,
            g.progress_text,
            g.video_path,
            g.thumbnail_path,
            g.first_frame_path,
            g.input_last_frame_path,
            g.returned_last_frame_path,
            (SELECT COUNT(*) FROM reference_images r WHERE r.generation_id = g.id) AS reference_count
        FROM generations g
        WHERE g.status IN ('queued', 'running')
        ORDER BY g.created_at DESC
        "#,
    )?;

    let items = stmt
        .query_map([], row_to_summary)?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(items)
}

pub fn get_generation_detail(conn: &Connection, generation_id: i64) -> Result<GenerationDetail> {
    let mut stmt = conn.prepare(
        r#"
        SELECT
            g.id,
            g.task_id,
            g.platform,
            g.model,
            g.status,
            g.prompt,
            g.prompt_summary,
            g.params_json,
            g.created_at,
            g.updated_at,
            g.completed_at,
            g.error_message,
            g.progress_text,
            g.video_path,
            g.thumbnail_path,
            g.first_frame_path,
            g.input_last_frame_path,
            g.returned_last_frame_path,
            (SELECT COUNT(*) FROM reference_images r WHERE r.generation_id = g.id) AS reference_count
        FROM generations g
        WHERE g.id = ?1
        "#,
    )?;

    let mut detail = stmt
        .query_row(params![generation_id], |row| {
            Ok(GenerationDetail {
                id: row.get(0)?,
                task_id: row.get(1)?,
                platform: row.get(2)?,
                model: row.get(3)?,
                status: row.get(4)?,
                prompt: row.get(5)?,
                prompt_summary: row.get(6)?,
                params_json: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
                completed_at: row.get(10)?,
                error_message: row.get(11)?,
                progress_text: row.get(12)?,
                video_path: row.get(13)?,
                thumbnail_path: row.get(14)?,
                first_frame_path: row.get(15)?,
                input_last_frame_path: row.get(16)?,
                returned_last_frame_path: row.get(17)?,
                reference_count: row.get::<_, i64>(18)? as usize,
                reference_images: Vec::new(),
            })
        })
        .with_context(|| format!("Generation {} not found", generation_id))?;

    let mut refs_stmt = conn.prepare(
        r#"
        SELECT image_path
        FROM reference_images
        WHERE generation_id = ?1
        ORDER BY sort_order ASC
        "#,
    )?;

    let reference_images = refs_stmt
        .query_map(params![generation_id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;

    detail.reference_images = reference_images;
    Ok(detail)
}

fn count_generations(conn: &Connection, status_filter: Option<&str>) -> Result<usize> {
    let count: i64 = if let Some(status) = status_filter {
        conn.query_row(
            "SELECT COUNT(*) FROM generations WHERE status = ?1",
            params![status],
            |row| row.get(0),
        )?
    } else {
        conn.query_row("SELECT COUNT(*) FROM generations", [], |row| row.get(0))?
    };

    Ok(count as usize)
}

fn row_to_summary(row: &Row<'_>) -> rusqlite::Result<GenerationSummary> {
    Ok(GenerationSummary {
        id: row.get(0)?,
        task_id: row.get(1)?,
        status: row.get(2)?,
        prompt: row.get(3)?,
        prompt_summary: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        completed_at: row.get(7)?,
        error_message: row.get(8)?,
        progress_text: row.get(9)?,
        video_path: row.get(10)?,
        thumbnail_path: row.get(11)?,
        first_frame_path: row.get(12)?,
        input_last_frame_path: row.get(13)?,
        returned_last_frame_path: row.get(14)?,
        reference_count: row.get::<_, i64>(15)? as usize,
    })
}
