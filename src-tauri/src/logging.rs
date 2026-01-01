use std::fs;
use std::path::PathBuf;
use chrono::{Local, NaiveDate};
use std::io;

use crate::{ paths, constants };

/// Initialize log directories and perform archiving
/// This should be called before initializing the Tauri log plugin
pub fn prepare_log_directories() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let log_dir = paths::get_logs_dir()?;
    let archive_dir = paths::get_logs_archive_dir()?;
    
    // Create log directories
    fs::create_dir_all(&log_dir)?;
    fs::create_dir_all(&archive_dir)?;
    
    // Archive old logs before starting
    archive_old_logs(&log_dir, &archive_dir)?;
    
    // Run initial cleanup
    if let Err(e) = cleanup_old_archives() {
        eprintln!("Warning: Failed to cleanup old log archives during init: {}", e);
    }
    
    Ok(log_dir)
}

/// Custom time formatter with 2 decimal places for seconds (kept for compatibility)
#[allow(dead_code)]
struct CustomTimeFormat;

#[allow(dead_code)]
impl tracing_subscriber::fmt::time::FormatTime for CustomTimeFormat {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        let now = Local::now();
        // Format base time without fractional seconds
        let base = now.format("%Y-%m-%d %H:%M:%S").to_string();
        // Get fractional seconds in centiseconds (hundredths)
        let centisecs = (now.timestamp_subsec_millis() / 10) % 100;
        w.write_str(&format!("{}.{:02}", base, centisecs))
    }
}

/// Build and return the Tauri log plugin builder
/// This replaces the old init_logging function
pub fn build_tauri_log_plugin() -> Result<tauri_plugin_log::Builder, Box<dyn std::error::Error>> {
    // Prepare directories and archive old logs
    let log_dir = prepare_log_directories()?;
    
    // Build the Tauri log plugin with custom configuration
    // Generate filename with today's date: sparrow.2025-12-23
    let today = Local::now().format("%Y-%m-%d").to_string();
    let log_filename = format!("sparrow.{}", today);
    
    let plugin_builder = tauri_plugin_log::Builder::new()
        // Configure log targets: Use Folder to write to our custom log directory
        .targets([
            tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Folder {
                path: log_dir.clone(),
                file_name: Some(log_filename), // Creates sparrow.YYYY-MM-DD.log files
            }),
            tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
        ])
        // Set log level (info and above)
        .level(log::LevelFilter::Info)
        // Debug level for our crate
        .level_for("sparrow_lib", log::LevelFilter::Debug)
        .level_for("SparrowAI", log::LevelFilter::Debug)
        // Reduce noise from dependencies
        .level_for("h2", log::LevelFilter::Warn)
        .level_for("hyper", log::LevelFilter::Warn)
        .level_for("reqwest", log::LevelFilter::Warn)
        .level_for("sled", log::LevelFilter::Warn)
        // Use local timezone for timestamps
        .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
        // Configure file rotation (keep all rotated files)
        .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepAll)
        // Set max file size (50MB)
        .max_file_size(50_000_000)
        // Custom format to simplify webview logs
        .format(|out, message, record| {
            // Simplify webview target format
            let target = record.target();
            let simplified_target = if target.starts_with("webview:") {
                // Remove function name and localhost URL from webview logs
                // From: webview:Logger.info@http://localhost:1420/src/lib/logger.ts:39:11
                // To:   webview:src/lib/logger.ts:39:11
                let parts: Vec<&str> = target.split('@').collect();
                if parts.len() > 1 {
                    let url_part = parts[1];
                    // Remove http://localhost:1420/ prefix
                    let simplified = url_part
                        .replace("http://localhost:1420/", "")
                        .replace("http://localhost:5173/", ""); // Also handle Vite dev server
                    format!("webview:{}", simplified)
                } else {
                    target.to_string()
                }
            } else {
                target.to_string()
            };
            
            out.finish(format_args!(
                "[{} {}] {}",
                record.level(),
                simplified_target,
                message
            ))
        });
    
    tracing::info!(
        log_dir = %log_dir.display(),
        "Tauri logging plugin configured"
    );
    
    Ok(plugin_builder)
}

/// Initialize the logging system with file-based logging and archiving
/// Legacy function - now calls build_tauri_log_plugin for compatibility
#[allow(dead_code)]
#[deprecated(note = "Use build_tauri_log_plugin() and register with Tauri builder instead")]
pub fn init_logging() -> Result<(), Box<dyn std::error::Error>> {
    // Just prepare directories for backward compatibility
    let log_dir = prepare_log_directories()?;
    
    eprintln!("🚀 SparrowAI starting...");
    eprintln!("📁 Log directory: {}", log_dir.display());
    eprintln!("⚠️  Please use build_tauri_log_plugin() with Tauri builder");
    
    Ok(())
}

/// Archive logs older than today
fn archive_old_logs(log_dir: &PathBuf, archive_dir: &PathBuf) -> io::Result<()> {
    let today = Local::now().naive_local().date();
    
    if !log_dir.exists() {
        return Ok(());
    }
    
    for entry in fs::read_dir(log_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        // Skip directories and non-log files
        if !path.is_file() || !path.extension().map_or(false, |ext| ext == "log") {
            continue;
        }
        
        let file_name = path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        
        // Parse date from filename (assuming format: sparrow.2024-01-01.log)
        if let Some(date_str) = extract_date_from_filename(file_name) {
            if let Ok(file_date) = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
                if file_date < today {
                    let archive_path = archive_dir.join(file_name);
                    match fs::rename(&path, &archive_path) {
                        Ok(_) => tracing::info!("Archived log file: {} -> {}", path.display(), archive_path.display()),
                        Err(e) => tracing::warn!("Failed to archive log file {}: {}", path.display(), e),
                    }
                }
            }
        }
    }
    
    Ok(())
}

/// Extract date from log filename
fn extract_date_from_filename(filename: &str) -> Option<String> {
    // Expected format: sparrow.2024-01-01.log
    let parts: Vec<&str> = filename.split('.').collect();
    if parts.len() >= 3 && parts[0] == "sparrow" {
        // Validate date format (YYYY-MM-DD)
        let date_part = parts[1];
        if date_part.len() == 10 && date_part.matches('-').count() == 2 {
            return Some(date_part.to_string());
        }
    }
    None
}

/// Clean up old archived logs (keep last 30 days)
pub fn cleanup_old_archives() -> Result<(), Box<dyn std::error::Error>> {
    let _log_dir = paths::get_logs_dir()?;
    let archive_dir = paths::get_logs_archive_dir()?;
    
    if !archive_dir.exists() {
        return Ok(());
    }
    
    let cutoff_date = Local::now().naive_local().date() - chrono::Duration::days(constants::LOG_RETENTION_DAYS);
    
    for entry in fs::read_dir(&archive_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        if !path.is_file() {
            continue;
        }
        
        let file_name = path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        
        if let Some(date_str) = extract_date_from_filename(file_name) {
            if let Ok(file_date) = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
                if file_date < cutoff_date {
                    match fs::remove_file(&path) {
                        Ok(_) => tracing::info!("Removed old archived log: {}", path.display()),
                        Err(e) => tracing::warn!("Failed to remove old archived log {}: {}", path.display(), e),
                    }
                }
            }
        }
    }
    
    Ok(())
}

/// Periodically clean up old archived logs (should be called periodically, e.g., daily)
/// This is a public function that can be called from the main application
pub async fn periodic_cleanup_task() {
    loop {
        // Wait 24 hours before next cleanup
        tokio::time::sleep(tokio::time::Duration::from_secs(24 * 60 * 60)).await;
        
        if let Err(e) = cleanup_old_archives() {
            tracing::warn!("Periodic log cleanup failed: {}", e);
        } else {
            tracing::debug!("Periodic log cleanup completed successfully");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_extract_date_from_filename() {
        assert_eq!(extract_date_from_filename("sparrow.2024-01-15.log"), Some("2024-01-15".to_string()));
        assert_eq!(extract_date_from_filename("sparrow.2024-12-31.log"), Some("2024-12-31".to_string()));
        assert_eq!(extract_date_from_filename("invalid-format.log"), None);
        assert_eq!(extract_date_from_filename("sparrow.log"), None);
        assert_eq!(extract_date_from_filename("other.2024-01-15.log"), None);
    }
}