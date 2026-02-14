use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::AppHandle;
use tracing::{debug, info};
use uuid::Uuid;
use zip::ZipArchive;

use crate::{chat, constants, paths, speech};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoTranscriptionResult {
    pub transcript: String,
    pub extracted_audio_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoAnalysisResult {
    pub answer: String,
    pub frame_paths: Vec<String>,
    pub frames_analyzed: usize,
    pub sample_interval_seconds: f32,
}

fn can_execute_ffmpeg(command_path: &Path) -> bool {
    let mut cmd = Command::new(command_path);
    cmd.arg("-version").stdout(Stdio::null()).stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    cmd.status().map(|s| s.success()).unwrap_or(false)
}

fn get_bundled_ffmpeg_path() -> Result<PathBuf, String> {
    let sparrow_dir = paths::get_sparrow_dir().map_err(|e| e.to_string())?;
    Ok(sparrow_dir
        .join("tools")
        .join("ffmpeg")
        .join(constants::FFMPEG_EXE_NAME))
}

#[cfg(target_os = "windows")]
async fn download_ffmpeg_windows(target_exe_path: &PathBuf) -> Result<(), String> {
    let target_dir = target_exe_path
        .parent()
        .ok_or_else(|| "Invalid FFmpeg target directory".to_string())?;

    fs::create_dir_all(target_dir)
        .map_err(|e| format!("Failed to create FFmpeg directory: {}", e))?;

    info!(
        url = %constants::FFMPEG_WINDOWS_DOWNLOAD_URL,
        target = %target_exe_path.display(),
        "Downloading portable FFmpeg"
    );

    let client = reqwest::Client::builder()
        .user_agent(constants::USER_AGENT)
        .timeout(std::time::Duration::from_secs(constants::DOWNLOAD_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(constants::FFMPEG_WINDOWS_DOWNLOAD_URL)
        .send()
        .await
        .map_err(|e| format!("Failed to download FFmpeg: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "FFmpeg download failed with status: {}",
            response.status()
        ));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read FFmpeg download data: {}", e))?;

    let cursor = Cursor::new(bytes);
    let mut archive = ZipArchive::new(cursor)
        .map_err(|e| format!("Failed to open FFmpeg zip archive: {}", e))?;

    let mut extracted = false;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read FFmpeg zip entry: {}", e))?;

        let name = file.name().to_lowercase();
        if name.ends_with("/bin/ffmpeg.exe") || name.ends_with("ffmpeg.exe") {
            let mut out_file = fs::File::create(target_exe_path)
                .map_err(|e| format!("Failed to create FFmpeg executable file: {}", e))?;
            std::io::copy(&mut file, &mut out_file)
                .map_err(|e| format!("Failed to extract FFmpeg executable: {}", e))?;
            extracted = true;
            break;
        }
    }

    if !extracted {
        return Err("Downloaded FFmpeg archive does not contain ffmpeg.exe".to_string());
    }

    Ok(())
}

async fn ensure_ffmpeg_available() -> Result<PathBuf, String> {
    // First try system PATH
    let system_ffmpeg = PathBuf::from("ffmpeg");
    if can_execute_ffmpeg(&system_ffmpeg) {
        return Ok(system_ffmpeg);
    }

    // Then try bundled location
    let bundled_ffmpeg = get_bundled_ffmpeg_path()?;
    if bundled_ffmpeg.exists() && can_execute_ffmpeg(&bundled_ffmpeg) {
        return Ok(bundled_ffmpeg);
    }

    #[cfg(target_os = "windows")]
    {
        // Auto-provision portable FFmpeg for Windows users
        download_ffmpeg_windows(&bundled_ffmpeg).await?;

        if bundled_ffmpeg.exists() && can_execute_ffmpeg(&bundled_ffmpeg) {
            info!(path = %bundled_ffmpeg.display(), "Using auto-downloaded FFmpeg");
            return Ok(bundled_ffmpeg);
        }

        return Err(
            "FFmpeg auto-download completed but executable validation failed. Please restart the app and try again.".to_string(),
        );
    }

    #[cfg(not(target_os = "windows"))]
    {
        Err(
            "ffmpeg is not available in PATH. Automatic provisioning is currently implemented for Windows only.".to_string(),
        )
    }
}

fn validate_video_file(video_file_path: &str) -> Result<(), String> {
    let path = Path::new(video_file_path);

    if !path.exists() {
        return Err(format!("Video file not found: {}", video_file_path));
    }

    if !path.is_file() {
        return Err(format!("Path is not a file: {}", video_file_path));
    }

    Ok(())
}

fn extract_audio_from_video(ffmpeg_path: &Path, video_file_path: &str) -> Result<PathBuf, String> {
    let audio_dir = paths::get_audio_dir().map_err(|e| e.to_string())?;
    let output_path = audio_dir.join(format!("video_audio_{}.wav", Uuid::new_v4()));

    let mut cmd = Command::new(ffmpeg_path);
    cmd.args([
        "-y",
        "-i",
        video_file_path,
        "-vn",
        "-ac",
        "1",
        "-ar",
        "16000",
        "-f",
        "wav",
        &output_path.to_string_lossy(),
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run ffmpeg for audio extraction: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to extract audio from video: {}", stderr.trim()));
    }

    Ok(output_path)
}

fn extract_frames_from_video(
    ffmpeg_path: &Path,
    video_file_path: &str,
    sample_interval_seconds: f32,
    max_frames: usize,
) -> Result<Vec<String>, String> {
    let interval = if sample_interval_seconds <= 0.0 {
        1.0
    } else {
        sample_interval_seconds
    };

    let temp_frames_dir = std::env::temp_dir().join(format!("sparrow_video_frames_{}", Uuid::new_v4()));
    fs::create_dir_all(&temp_frames_dir)
        .map_err(|e| format!("Failed to create temporary frames directory: {}", e))?;

    let frame_pattern = temp_frames_dir.join("frame_%06d.jpg");
    let fps_filter = format!("fps=1/{}", interval);

    let mut cmd = Command::new(ffmpeg_path);
    cmd.args([
        "-y",
        "-i",
        video_file_path,
        "-vf",
        &fps_filter,
        "-q:v",
        "3",
        &frame_pattern.to_string_lossy(),
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run ffmpeg for frame extraction: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to extract frames from video: {}", stderr.trim()));
    }

    let mut frame_paths: Vec<String> = fs::read_dir(&temp_frames_dir)
        .map_err(|e| format!("Failed to read extracted frames directory: {}", e))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jpg"))
        .map(|path| path.to_string_lossy().to_string())
        .collect();

    frame_paths.sort();

    if frame_paths.is_empty() {
        return Err("No frames were extracted from the video. Please verify the input video format.".to_string());
    }

    if frame_paths.len() > max_frames {
        frame_paths.truncate(max_frames);
    }

    Ok(frame_paths)
}

#[tauri::command]
pub async fn transcribe_video(
    video_file_path: String,
    model_id: String,
    language: Option<String>,
) -> Result<VideoTranscriptionResult, String> {
    info!(video_file = %video_file_path, model = %model_id, "Starting video transcription");

    let ffmpeg_path = ensure_ffmpeg_available().await?;
    validate_video_file(&video_file_path)?;

    let audio_path = extract_audio_from_video(&ffmpeg_path, &video_file_path)?;
    let audio_path_str = audio_path.to_string_lossy().to_string();

    let transcript = speech::transcribe_audio(audio_path_str.clone(), model_id, language).await?;

    Ok(VideoTranscriptionResult {
        transcript,
        extracted_audio_path: audio_path_str,
    })
}

#[tauri::command]
pub async fn analyze_video_content(
    app: AppHandle,
    video_file_path: String,
    model_name: String,
    query: String,
    sample_interval_seconds: Option<f32>,
    max_frames: Option<usize>,
) -> Result<VideoAnalysisResult, String> {
    info!(
        video_file = %video_file_path,
        model = %model_name,
        query_len = query.len(),
        "Starting video content analysis"
    );

    let ffmpeg_path = ensure_ffmpeg_available().await?;
    validate_video_file(&video_file_path)?;

    let sample_interval = sample_interval_seconds.unwrap_or(1.0);
    let max_frames = max_frames.unwrap_or(8).clamp(1, 24);

    let frame_paths = extract_frames_from_video(
        &ffmpeg_path,
        &video_file_path,
        sample_interval,
        max_frames,
    )?;

    debug!(frames = frame_paths.len(), "Frames extracted for video analysis");

    let attachments: Vec<chat::AttachmentInfo> = frame_paths
        .iter()
        .map(|path| chat::AttachmentInfo {
            file_path: path.clone(),
            file_name: Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("frame.jpg")
                .to_string(),
            file_type: "jpg".to_string(),
            is_image: true,
            is_video: false,
        })
        .collect();

    let analysis_prompt = format!(
        "You are analyzing sampled frames from a video in chronological order. \
Use the provided images to answer the user request. \
If uncertain, say so explicitly.\n\nUser request:\n{}",
        query
    );

    let answer = chat::chat_with_loaded_model_streaming(
        app,
        model_name,
        analysis_prompt,
        None,
        Some(false),
        Some("You are a precise video analysis assistant. Focus only on visual evidence from the provided frames.".to_string()),
        Some(0.2),
        Some(0.95),
        None,
        Some(1000),
        None,
        Some(attachments),
    )
    .await?;

    Ok(VideoAnalysisResult {
        answer,
        frame_paths: frame_paths.clone(),
        frames_analyzed: frame_paths.len(),
        sample_interval_seconds: sample_interval,
    })
}

#[tauri::command]
pub async fn query_video_for_object(
    app: AppHandle,
    video_file_path: String,
    model_name: String,
    object_name: String,
    sample_interval_seconds: Option<f32>,
    max_frames: Option<usize>,
) -> Result<VideoAnalysisResult, String> {
    let query = format!(
        "Are there any {} in the video? If yes, describe where/when they appear and key visual details.",
        object_name
    );

    analyze_video_content(
        app,
        video_file_path,
        model_name,
        query,
        sample_interval_seconds,
        max_frames,
    )
    .await
}
