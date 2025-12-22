use std::fs;
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, fmt};
use tracing_appender::{non_blocking, rolling};
use chrono::{Local, NaiveDate};
use std::io;

use crate::{ paths, constants };

/// Custom time formatter with 2 decimal places for seconds
struct CustomTimeFormat;

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

/// Initialize the logging system with file-based logging and archiving
pub fn init_logging() -> Result<(), Box<dyn std::error::Error>> {
    let log_dir = paths::get_logs_dir()?;
    let archive_dir = paths::get_logs_archive_dir()?;
    
    // Create log directories
    fs::create_dir_all(&log_dir)?;
    fs::create_dir_all(&archive_dir)?;
    
    // Archive old logs before starting
    archive_old_logs(&log_dir, &archive_dir)?;
    
    // Set up file appender for daily rotation with .log extension
    let file_appender = rolling::Builder::new()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix("sparrow")
        .filename_suffix("log")
        .build(&log_dir)
        .map_err(|e| format!("Failed to create rolling file appender: {}", e))?;
    let (non_blocking_appender, _guard) = non_blocking(file_appender);
    
    // Create console layer - clean, user-friendly output
    // Format: [LEVEL] message (for INFO and above)
    // Errors show more context
    let console_layer = fmt::layer()
        .with_target(false)
        .with_thread_ids(false)
        .with_line_number(false)
        .with_file(false)
        .with_level(true) // Show level for user awareness
        .with_ansi(true)
        .compact() // Use compact format for cleaner output
        .with_timer(CustomTimeFormat);
    
    // Create file layer - detailed, structured logging for debugging
    // Format: timestamp [LEVEL] target:line_number - message {fields}
    let file_layer = fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .with_line_number(true)
        .with_file(true)
        .with_ansi(false)
        .with_timer(CustomTimeFormat)
        .with_writer(non_blocking_appender);
    
    // Set up environment filter
    // Console: INFO and above (clean output)
    // File: DEBUG and above (detailed logging)
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            EnvFilter::new(constants::DEFAULT_LOG_FILTER)
                // Reduce noise from dependencies in console
                .add_directive("h2=warn".parse().unwrap())
                .add_directive("hyper=warn".parse().unwrap())
                .add_directive("reqwest=warn".parse().unwrap())
                .add_directive("sled=warn".parse().unwrap())
        });
    
    // Initialize the subscriber
    tracing_subscriber::registry()
        .with(env_filter)
        .with(console_layer)
        .with(file_layer)
        .init();
    
    tracing::info!("🚀 SparrowAI starting...");
    tracing::debug!(
        log_dir = %log_dir.display(),
        archive_dir = %archive_dir.display(),
        "Logging system initialized"
    );
    
    // Store the guard to prevent dropping (this keeps the non-blocking writer alive)
    std::mem::forget(_guard);
    
    // Run initial cleanup synchronously (don't spawn tokio task here)
    if let Err(e) = cleanup_old_archives() {
        tracing::warn!("Failed to cleanup old log archives during init: {}", e);
    }
    
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