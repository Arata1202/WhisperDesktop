use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::config::Region;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::Client;
use chrono::{NaiveTime, Timelike};
use directories::{ProjectDirs, UserDirs};
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::fs;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct MinioConfig {
    url: String,
    #[serde(alias = "access_key")]
    access_key: String,
    #[serde(alias = "secret_key")]
    secret_key: String,
    bucket: String,
    region: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct WhisperConfig {
    #[serde(alias = "binary_path")]
    binary_path: String,
    #[serde(alias = "model_path")]
    model_path: String,
    #[serde(alias = "output_dir")]
    output_dir: String,
    #[serde(alias = "include_timestamps")]
    include_timestamps: bool,
    #[serde(alias = "include_speaker")]
    include_speaker: bool,
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            binary_path: String::new(),
            model_path: String::new(),
            output_dir: String::new(),
            include_timestamps: false,
            include_speaker: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct AppConfig {
    minio: MinioConfig,
    whisper: WhisperConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MeetingSummary {
    id: String,
    date: String,
    room_id: String,
    meeting_time: String,
    speaker_count: usize,
    track_count: usize,
}

#[derive(Debug, Clone)]
struct TrackEntry {
    key: String,
    speaker: String,
    track_time: String,
}

#[derive(Debug, Deserialize)]
struct WhisperSegment {
    start: f64,
    text: String,
}

#[derive(Debug, Deserialize)]
struct WhisperJson {
    segments: Vec<WhisperSegment>,
}

#[derive(Debug, Clone)]
struct TranscriptionSegment {
    start: f64,
    speaker: String,
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobStatus {
    state: String,
    completed: usize,
    total: usize,
    output_path: Option<String>,
    error: Option<String>,
    log: Option<String>,
}

type JobState = std::sync::Arc<Mutex<HashMap<String, JobStatus>>>;

fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("com", "whisperdesktop", "WhisperDesktop")
        .ok_or_else(|| anyhow!("Failed to resolve config directory"))
}

async fn effective_config() -> Result<AppConfig> {
    load_saved_config().await
}

fn config_path() -> Result<PathBuf> {
    let dirs = project_dirs()?;
    Ok(dirs.config_dir().join("config.json"))
}

async fn load_saved_config() -> Result<AppConfig> {
    let path = config_path()?;
    match fs::read_to_string(&path).await {
        Ok(contents) => {
            let trimmed = contents.trim();
            if trimmed.is_empty() {
                return Ok(AppConfig::default());
            }
            let config: AppConfig = serde_json::from_str(trimmed)?;
            Ok(config)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(err) => Err(err.into()),
    }
}

async fn save_config_file(config: &AppConfig) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let payload = serde_json::to_string_pretty(config)?;
    fs::write(path, payload).await?;
    Ok(())
}


async fn s3_client(config: &AppConfig) -> Result<Client> {
    let minio = &config.minio;
    if minio.url.is_empty()
        || minio.access_key.is_empty()
        || minio.secret_key.is_empty()
        || minio.bucket.is_empty()
    {
        return Err(anyhow!("MinIO config is incomplete"));
    }

    let region = if minio.region.is_empty() {
        Region::new("us-east-1")
    } else {
        Region::new(minio.region.clone())
    };

    let creds = Credentials::new(
        minio.access_key.clone(),
        minio.secret_key.clone(),
        None,
        None,
        "static",
    );

    let shared = aws_config::defaults(BehaviorVersion::latest())
        .region(region)
        .credentials_provider(creds)
        .load()
        .await;

    let conf = aws_sdk_s3::config::Builder::from(&shared)
        .endpoint_url(minio.url.clone())
        .force_path_style(true)
        .build();

    Ok(Client::from_conf(conf))
}

#[tauri::command]
async fn check_minio() -> Result<(), String> {
    let config = effective_config().await.map_err(|err| err.to_string())?;
    let client = s3_client(&config).await.map_err(|err| err.to_string())?;
    client
        .list_objects_v2()
        .bucket(&config.minio.bucket)
        .max_keys(1)
        .send()
        .await
        .map_err(format_sdk_error)?;
    Ok(())
}

fn format_sdk_error<E: std::fmt::Debug>(err: SdkError<E>) -> String {
    format!("{err:?}")
}

fn parse_key(key: &str) -> Option<(String, String, String, String, String)> {
    let mut parts = key.split('/');
    let date = parts.next()?.to_string();
    let room_id = parts.next()?.to_string();
    let meeting_time = parts.next()?.to_string();
    let speaker = parts.next()?.to_string();
    let file = parts.next()?.to_string();

    if parts.next().is_some() {
        return None;
    }

    let file = file.strip_suffix(".ogg").unwrap_or(&file);
    let (track_time, _) = match file.split_once('_') {
        Some((time, rest)) => (time.to_string(), rest.to_string()),
        None => (file.to_string(), String::new()),
    };

    Some((date, room_id, meeting_time, speaker, track_time))
}

fn sanitize_time(value: &str) -> String {
    if NaiveTime::parse_from_str(value, "%H-%M-%S").is_ok() {
        value.to_string()
    } else {
        value.to_string()
    }
}

fn output_root(config: &AppConfig) -> Result<PathBuf> {
    if !config.whisper.output_dir.trim().is_empty() {
        return Ok(PathBuf::from(config.whisper.output_dir.trim()));
    }
    default_output_dir()
}

fn default_output_dir() -> Result<PathBuf> {
    if let Some(user_dirs) = UserDirs::new() {
        if let Some(downloads) = user_dirs.download_dir() {
            return Ok(downloads.to_path_buf());
        }
    }
    let dirs = project_dirs()?;
    Ok(dirs.data_dir().join("transcripts"))
}

fn whisper_base_dir() -> Result<PathBuf> {
    let dirs = project_dirs()?;
    Ok(dirs.data_dir().join("whisper"))
}

fn default_whisper_binary_candidates() -> Vec<&'static str> {
    if cfg!(target_os = "windows") {
        vec!["whisper.exe", "whisper-cpp.exe", "main.exe"]
    } else {
        vec!["whisper-cli", "whisper", "whisper-cpp", "main"]
    }
}

fn find_in_path(binary: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn resolve_whisper_paths(config: &AppConfig) -> Result<(PathBuf, PathBuf)> {
    let base_dir = whisper_base_dir()?;
    let requested_binary = config.whisper.binary_path.trim();
    let binary = if requested_binary.is_empty() {
        let mut found: Option<PathBuf> = None;
        for candidate in default_whisper_binary_candidates() {
            if let Some(path) = find_in_path(candidate) {
                found = Some(path);
                break;
            }
        }
        found.ok_or_else(|| {
            anyhow!(
                "whisper binary not found in PATH. Install whisper.cpp or set WHISPER_BINARY."
            )
        })?
    } else {
        let requested = PathBuf::from(requested_binary);
        if requested.is_absolute() || requested.exists() {
            requested
        } else if let Some(found) = find_in_path(requested_binary) {
            found
        } else {
            requested
        }
    };
    let requested_model = config.whisper.model_path.trim();
    let model = if requested_model.is_empty() {
        base_dir.join("models").join("ggml-large-v3.bin")
    } else {
        let requested_path = PathBuf::from(requested_model);
        if requested_path.is_absolute() {
            requested_path
        } else if requested_model.starts_with("models/") || requested_model.starts_with("models\\")
        {
            base_dir.join(requested_model)
        } else {
            base_dir.join("models").join(requested_model)
        }
    };
    Ok((binary, model))
}

fn append_log(jobs_state: &JobState, job_id: &str, line: &str) {
    let mut map = jobs_state.lock().unwrap();
    if let Some(status) = map.get_mut(job_id) {
        let log = status.log.get_or_insert_with(String::new);
        log.push_str(line);
        log.push('\n');
    }
}

async fn ensure_whisper_resources(config: &AppConfig) -> Result<(PathBuf, PathBuf)> {
    let (binary_path, model_path) = resolve_whisper_paths(config)?;
    if !binary_path.exists() {
        let hint = if config.whisper.binary_path.trim().is_empty() {
            format!(
                "Install whisper.cpp and ensure one of {:?} is in PATH.",
                default_whisper_binary_candidates()
            )
        } else {
            "Set WHISPER_BINARY to a valid local path.".to_string()
        };
        return Err(anyhow!(
            "Whisper binary not found at {}. {}",
            binary_path.display(),
            hint
        ));
    }

    if !model_path.exists() {
        return Err(anyhow!(
            "Whisper model not found at {}. Set WHISPER_MODEL to a local model file.",
            model_path.display()
        ));
    }

    Ok((binary_path, model_path))
}

async fn download_object(client: &Client, bucket: &str, key: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).await?;
    }
    let obj = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .with_context(|| format!("Failed to download {key}"))?;
    let data = obj
        .body
        .collect()
        .await
        .with_context(|| "Failed to read object stream")?
        .into_bytes();
    fs::write(dest, data)
        .await
        .with_context(|| format!("Failed to write file: {}", dest.display()))?;
    Ok(())
}

async fn run_whisper_segments(
    binary_path: &Path,
    model_path: &Path,
    input: &Path,
    output_base: &Path,
    jobs_state: &JobState,
    job_id: &str,
) -> Result<Vec<WhisperSegment>> {
    let output_base_str = output_base.to_string_lossy().to_string();
    let mut child = Command::new(binary_path)
        .arg("-m")
        .arg(model_path)
        .arg("-f")
        .arg(input)
        .arg("-l")
        .arg("ja")
        .arg("-oj")
        .arg("-otxt")
        .arg("-of")
        .arg(&output_base_str)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| "Failed to execute whisper")?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Failed to capture whisper stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("Failed to capture whisper stderr"))?;
    let stdout_state = jobs_state.clone();
    let stdout_job = job_id.to_string();
    let stderr_state = jobs_state.clone();
    let stderr_job = job_id.to_string();
    let stdout_task = tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        while let Some(line) = lines.next_line().await? {
            if !line.trim().is_empty() {
                append_log(&stdout_state, &stdout_job, &line);
            }
        }
        Ok::<(), anyhow::Error>(())
    });
    let stderr_task = tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        while let Some(line) = lines.next_line().await? {
            if !line.trim().is_empty() {
                append_log(&stderr_state, &stderr_job, &line);
            }
        }
        Ok::<(), anyhow::Error>(())
    });

    let status = child.wait().await?;
    let _ = stdout_task.await;
    let _ = stderr_task.await;

    if !status.success() {
        return Err(anyhow!("Whisper command failed"));
    }

    let json_path = output_base.with_extension("json");
    let json = fs::read_to_string(&json_path)
        .await
        .with_context(|| format!("Failed to read whisper output: {}", json_path.display()))?;
    let json = normalize_json_contents(&json);
    if let Ok(parsed) = serde_json::from_str::<WhisperJson>(&json) {
        return Ok(parsed.segments);
    }
    if let Ok(parsed) = serde_json::from_str::<Vec<WhisperSegment>>(&json) {
        return Ok(parsed);
    }

    if let Some(segments) = parse_json_lines(&json) {
        return Ok(segments);
    }

    let value: serde_json::Value =
        serde_json::from_str(&json).with_context(|| "Failed to parse whisper JSON output")?;
    if let Some(segments) = extract_segments_from_value(value) {
        return Ok(segments);
    }

    if let Some(segments) = parse_json_lines(&json) {
        return Ok(segments);
    }

    let txt_path = output_base.with_extension("txt");
    if let Ok(text) = fs::read_to_string(&txt_path).await {
        let cleaned = text
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if !cleaned.is_empty() {
            eprintln!("whisper json parse failed; using txt fallback");
            return Ok(vec![WhisperSegment {
                start: 0.0,
                text: cleaned,
            }]);
        }
    }

    Err(anyhow!("Failed to parse whisper JSON output"))
}

fn is_wav(path: &Path) -> bool {
    path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("wav"))
        .unwrap_or(false)
}

async fn convert_to_wav(input: &Path, output: &Path) -> Result<()> {
    let ffmpeg = std::env::var("FFMPEG_BINARY").unwrap_or_else(|_| "ffmpeg".to_string());
    let ffmpeg_path = Path::new(&ffmpeg);
    let resolved = if ffmpeg_path.components().count() > 1 {
        if !ffmpeg_path.exists() {
            return Err(anyhow!("ffmpeg not found at {}", ffmpeg_path.display()));
        }
        ffmpeg_path.to_path_buf()
    } else if let Some(found) = find_in_path(&ffmpeg) {
        found
    } else {
        return Err(anyhow!(
            "ffmpeg not found in PATH. Install ffmpeg or set FFMPEG_BINARY."
        ));
    };
    let output_result = Command::new(&resolved)
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-ar")
        .arg("16000")
        .arg("-ac")
        .arg("1")
        .arg(output)
        .output()
        .await
        .with_context(|| format!("Failed to execute ffmpeg: {}", resolved.display()))?;

    if !output_result.status.success() {
        let stderr = String::from_utf8_lossy(&output_result.stderr);
        return Err(anyhow!("ffmpeg failed: {stderr}"));
    }

    Ok(())
}

fn extract_segments_from_value(value: serde_json::Value) -> Option<Vec<WhisperSegment>> {
    if let Some(segments) = value.get("segments") {
        return segments.as_array().and_then(segments_from_array);
    }
    if let Some(transcription) = value.get("transcription") {
        return transcription.as_array().and_then(segments_from_array);
    }
    if let Some(results) = value.get("results") {
        if let Some(segments) = results.get("segments") {
            return segments.as_array().and_then(segments_from_array);
        }
    }
    if let Some(array) = value.as_array() {
        return segments_from_array(array);
    }
    None
}

fn segments_from_array(items: &Vec<serde_json::Value>) -> Option<Vec<WhisperSegment>> {
    let mut segments = Vec::new();
    for item in items {
        if let Some(segment) = segment_from_value(item) {
            segments.push(segment);
        }
    }
    if segments.is_empty() {
        None
    } else {
        Some(segments)
    }
}

fn segment_from_value(value: &serde_json::Value) -> Option<WhisperSegment> {
    let obj = value.as_object()?;
    let text = obj
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if text.trim().is_empty() {
        return None;
    }
    let start = if let Some(start) = obj.get("start").and_then(|v| v.as_f64()) {
        start
    } else if let Some(offsets) = obj.get("offsets") {
        offsets
            .get("from")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            / 1000.0
    } else if let Some(timestamps) = obj.get("timestamps") {
        timestamps
            .get("from")
            .and_then(|v| v.as_str())
            .and_then(parse_timestamp_to_seconds)
            .unwrap_or(0.0)
    } else if let Some(t0) = obj.get("t0").and_then(|v| v.as_f64()) {
        t0 / 100.0
    } else {
        0.0
    };

    Some(WhisperSegment { start, text })
}

fn parse_json_lines(contents: &str) -> Option<Vec<WhisperSegment>> {
    let mut segments = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if let Some(list) = extract_segments_from_value(value.clone()) {
            segments.extend(list);
            continue;
        }
        if let Some(segment) = segment_from_value(&value) {
            segments.push(segment);
        }
    }
    if segments.is_empty() {
        None
    } else {
        Some(segments)
    }
}

fn parse_timestamp_to_seconds(value: &str) -> Option<f64> {
    let mut parts = value.split(':');
    let hours: f64 = parts.next()?.parse().ok()?;
    let minutes: f64 = parts.next()?.parse().ok()?;
    let seconds_part = parts.next()?;
    let mut sec_parts = seconds_part.split(',');
    let seconds: f64 = sec_parts.next()?.parse().ok()?;
    let millis: f64 = sec_parts.next().unwrap_or("0").parse().unwrap_or(0.0);
    Some(hours * 3600.0 + minutes * 60.0 + seconds + millis / 1000.0)
}

fn normalize_json_contents(contents: &str) -> String {
    let trimmed = contents.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let start = trimmed.find(|c| c == '{' || c == '[');
    let end = trimmed.rfind(|c| c == '}' || c == ']');
    match (start, end) {
        (Some(start), Some(end)) if end >= start => trimmed[start..=end].to_string(),
        _ => trimmed.to_string(),
    }
}

fn format_seconds(value: f64) -> String {
    let total = value.round().max(0.0) as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn format_segments(
    segments: &[TranscriptionSegment],
    include_timestamps: bool,
    include_speaker: bool,
) -> String {
    let mut output = String::new();
    for segment in segments {
        if include_timestamps {
            if include_speaker {
                output.push_str(&format!(
                    "{} {}：{}\n",
                    format_seconds(segment.start),
                    segment.speaker,
                    segment.text
                ));
            } else {
                output.push_str(&format!(
                    "{} {}\n",
                    format_seconds(segment.start),
                    segment.text
                ));
            }
        } else if include_speaker {
            output.push_str(&format!("{}：{}\n", segment.speaker, segment.text));
        } else {
            output.push_str(&format!("{}\n", segment.text));
        }
    }
    output
}

#[tauri::command]
async fn list_dates() -> Result<Vec<String>, String> {
    let config = effective_config().await.map_err(|err| err.to_string())?;
    let client = s3_client(&config).await.map_err(|err| err.to_string())?;

    let mut dates = Vec::new();
    let mut continuation: Option<String> = None;
    let mut saw_prefixes = false;
    loop {
        let mut req = client
            .list_objects_v2()
            .bucket(&config.minio.bucket)
            .delimiter("/");
        if let Some(token) = &continuation {
            req = req.continuation_token(token);
        }
        let resp = req.send().await.map_err(format_sdk_error)?;

        for prefix in resp.common_prefixes() {
            saw_prefixes = true;
            if let Some(value) = prefix.prefix() {
                let trimmed = value.trim_end_matches('/');
                if !trimmed.is_empty() {
                    dates.push(trimmed.to_string());
                }
            }
        }

        if resp.is_truncated().unwrap_or(false) {
            continuation = resp.next_continuation_token().map(|s| s.to_string());
            if continuation.is_none() {
                break;
            }
        } else {
            break;
        }
    }

    if !saw_prefixes {
        let mut continuation: Option<String> = None;
        loop {
            let mut req = client.list_objects_v2().bucket(&config.minio.bucket);
            if let Some(token) = &continuation {
                req = req.continuation_token(token);
            }
            let resp = req.send().await.map_err(format_sdk_error)?;
            for object in resp.contents() {
                if let Some(key) = object.key() {
                    if let Some(date) = key.split('/').next() {
                        if !date.is_empty() {
                            dates.push(date.to_string());
                        }
                    }
                }
            }
            if resp.is_truncated().unwrap_or(false) {
                continuation = resp.next_continuation_token().map(|s| s.to_string());
                if continuation.is_none() {
                    break;
                }
            } else {
                break;
            }
        }
    }

    dates.sort();
    dates.dedup();
    dates.sort();
    Ok(dates)
}

#[tauri::command]
async fn list_meetings(date: String) -> Result<Vec<MeetingSummary>, String> {
    let config = effective_config().await.map_err(|err| err.to_string())?;
    let client = s3_client(&config).await.map_err(|err| err.to_string())?;

    let prefix = format!("{date}/");
    let mut meetings: HashMap<String, (String, String, String, HashSet<String>, usize)> =
        HashMap::new();

    let mut continuation: Option<String> = None;
    loop {
        let mut req = client
            .list_objects_v2()
            .bucket(&config.minio.bucket)
            .prefix(prefix.clone());
        if let Some(token) = &continuation {
            req = req.continuation_token(token);
        }
        let resp = req.send().await.map_err(format_sdk_error)?;

        for object in resp.contents() {
            if let Some(key) = object.key() {
                if let Some((date, room_id, meeting_time, speaker, _)) = parse_key(key) {
                    let meeting_id = format!("{}/{}/{}", date, room_id, meeting_time);
                    let entry = meetings
                        .entry(meeting_id.clone())
                        .or_insert((date, room_id, meeting_time, HashSet::new(), 0));
                    entry.3.insert(speaker);
                    entry.4 += 1;
                }
            }
        }

        if resp.is_truncated().unwrap_or(false) {
            continuation = resp.next_continuation_token().map(|s| s.to_string());
            if continuation.is_none() {
                break;
            }
        } else {
            break;
        }
    }

    let mut list: Vec<MeetingSummary> = meetings
        .into_iter()
        .map(
            |(id, (date, room_id, meeting_time, speakers, track_count))| MeetingSummary {
                id,
                date,
                room_id,
                meeting_time,
                speaker_count: speakers.len(),
                track_count,
            },
        )
        .collect();

    list.sort_by(|a, b| b.meeting_time.cmp(&a.meeting_time));
    Ok(list)
}

#[tauri::command]
async fn start_transcribe(meeting_id: String, jobs: State<'_, JobState>) -> Result<String, String> {
    let config = effective_config().await.map_err(|err| err.to_string())?;
    let client = s3_client(&config).await.map_err(|err| err.to_string())?;

    let job_id = Uuid::new_v4().to_string();
    let mut map = jobs.lock().unwrap();
    map.insert(
        job_id.clone(),
        JobStatus {
            state: "running".to_string(),
            completed: 0,
            total: 0,
            output_path: None,
            error: None,
            log: Some(String::new()),
        },
    );
    drop(map);

    let jobs_state = jobs.inner().clone();
    let config_for_task = config.clone();
    let client_for_task = client.clone();
    let job_id_for_task = job_id.clone();
    let meeting_id_for_task = meeting_id.clone();
    tokio::spawn(async move {
        if let Err(err) = run_transcription(
            &config_for_task,
            &client_for_task,
            &meeting_id_for_task,
            &job_id_for_task,
            &jobs_state,
        )
        .await
        {
            let mut map = jobs_state.lock().unwrap();
            if let Some(status) = map.get_mut(&job_id_for_task) {
                status.state = "failed".to_string();
                status.error = Some(err.to_string());
            }
        }
    });

    Ok(job_id)
}

async fn run_transcription(
    config: &AppConfig,
    client: &Client,
    meeting_id: &str,
    job_id: &str,
    jobs_state: &JobState,
) -> Result<()> {
    let (binary_path, model_path) = ensure_whisper_resources(config).await?;
    let prefix = format!("{}/", meeting_id);
    let mut tracks = Vec::new();
    let mut continuation: Option<String> = None;
    loop {
        let mut req = client
            .list_objects_v2()
            .bucket(&config.minio.bucket)
            .prefix(prefix.clone());
        if let Some(token) = &continuation {
            req = req.continuation_token(token);
        }
        let resp = req
            .send()
            .await
            .map_err(|err| anyhow!(format_sdk_error(err)))?;

        for object in resp.contents() {
            if let Some(key) = object.key() {
                if let Some((_, _, _, speaker, track_time)) = parse_key(key) {
                    tracks.push(TrackEntry {
                        key: key.to_string(),
                        speaker,
                        track_time: sanitize_time(&track_time),
                    });
                }
            }
        }

        if resp.is_truncated().unwrap_or(false) {
            continuation = resp.next_continuation_token().map(|s| s.to_string());
            if continuation.is_none() {
                break;
            }
        } else {
            break;
        }
    }

    tracks.sort_by(|a, b| a.track_time.cmp(&b.track_time));
    eprintln!(
        "run_transcription meeting_id={} tracks_found={}",
        meeting_id,
        tracks.len()
    );

    {
        let mut map = jobs_state.lock().unwrap();
        if let Some(status) = map.get_mut(job_id) {
            status.total = tracks.len();
            status.completed = 0;
        }
    }

    if tracks.is_empty() {
        return Err(anyhow!("No tracks found for meeting: {meeting_id}"));
    }

    let output_root = output_root(config)?;
    let output_name = meeting_id.replace(['/', '\\'], "_");
    let output_path = output_root.join(output_name).with_extension("txt");
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Failed to create output dir: {}", parent.display()))?;
    }

    let temp_root = std::env::temp_dir().join("whisperdesktop").join(job_id);
    fs::create_dir_all(&temp_root).await?;

    let mut all_segments: Vec<TranscriptionSegment> = Vec::new();
    let include_timestamps = config.whisper.include_timestamps;
    let include_speaker = config.whisper.include_speaker;

    for (index, track) in tracks.iter().enumerate() {
        let local_file = temp_root.join(format!("track_{index}.ogg"));
        download_object(client, &config.minio.bucket, &track.key, &local_file).await?;

        let output_base = temp_root.join(format!("out_{index}"));
        let input_for_whisper = if is_wav(&local_file) {
            local_file.clone()
        } else {
            let wav_path = temp_root.join(format!("track_{index}.wav"));
            convert_to_wav(&local_file, &wav_path).await?;
            wav_path
        };
        let segments = run_whisper_segments(
            &binary_path,
            &model_path,
            &input_for_whisper,
            &output_base,
            jobs_state,
            job_id,
        )
        .await?;
        let track_start_seconds = NaiveTime::parse_from_str(&track.track_time, "%H-%M-%S")
            .map(|t| t.num_seconds_from_midnight() as f64)
            .unwrap_or(0.0);
        let mut track_segments: Vec<TranscriptionSegment> = Vec::new();
        for segment in segments {
            let cleaned = segment.text.trim();
            if cleaned.is_empty() {
                continue;
            }
            let start_abs = track_start_seconds + segment.start;
            track_segments.push(TranscriptionSegment {
                start: start_abs,
                speaker: track.speaker.clone(),
                text: cleaned.to_string(),
            });
        }

        track_segments.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all_segments.extend(track_segments.iter().cloned());
        let mut map = jobs_state.lock().unwrap();
        if let Some(status) = map.get_mut(job_id) {
            status.completed = index + 1;
        }
    }

    all_segments.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let output = format_segments(&all_segments, include_timestamps, include_speaker);

    fs::write(&output_path, output)
        .await
        .with_context(|| format!("Failed to write output: {}", output_path.display()))?;

    let mut map = jobs_state.lock().unwrap();
    if let Some(status) = map.get_mut(job_id) {
        status.state = "done".to_string();
        status.output_path = Some(output_path.to_string_lossy().to_string());
    }
    append_log(jobs_state, job_id, "Done");

    Ok(())
}

#[tauri::command]
async fn get_transcribe_status(
    job_id: String,
    jobs: State<'_, JobState>,
) -> Result<JobStatus, String> {
    let map = jobs.lock().unwrap();
    map.get(&job_id)
        .cloned()
        .ok_or_else(|| "Job not found".to_string())
}

#[tauri::command]
async fn get_config() -> Result<AppConfig, String> {
    load_saved_config().await.map_err(|err| err.to_string())
}

#[tauri::command]
async fn set_config(config: AppConfig) -> Result<(), String> {
    save_config_file(&config)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn get_default_output_dir() -> Result<String, String> {
    default_output_dir()
        .map(|path| path.to_string_lossy().to_string())
        .map_err(|err| err.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(std::sync::Arc::new(Mutex::new(
            HashMap::<String, JobStatus>::new(),
        )))
        .invoke_handler(tauri::generate_handler![
            list_dates,
            list_meetings,
            start_transcribe,
            get_transcribe_status,
            get_config,
            set_config,
            get_default_output_dir,
            check_minio
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
