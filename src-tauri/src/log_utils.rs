/// Logging utilities for consistent, informative logging throughout the application
/// 
/// This module provides helper macros and functions for structured logging
/// with appropriate context for debugging while keeping console output clean.

/// Log the start of a long-running operation
#[macro_export]
macro_rules! log_operation_start {
    ($op_name:expr) => {
        tracing::info!("▶️  {}", $op_name);
        tracing::debug!(operation = $op_name, "Operation started");
    };
    ($op_name:expr, $($field:tt)*) => {
        tracing::info!("▶️  {}", $op_name);
        tracing::debug!(operation = $op_name, $($field)*, "Operation started");
    };
}

/// Log successful completion of an operation
#[macro_export]
macro_rules! log_operation_success {
    ($op_name:expr) => {
        tracing::info!("✅ {}", $op_name);
        tracing::debug!(operation = $op_name, "Operation completed successfully");
    };
    ($op_name:expr, $($field:tt)*) => {
        tracing::info!("✅ {}", $op_name);
        tracing::debug!(operation = $op_name, $($field)*, "Operation completed successfully");
    };
}

/// Log operation failure with error context
#[macro_export]
macro_rules! log_operation_error {
    ($op_name:expr, $error:expr) => {
        tracing::error!("❌ {} failed: {}", $op_name, $error);
        tracing::debug!(
            operation = $op_name,
            error = %$error,
            "Operation failed"
        );
    };
    ($op_name:expr, $error:expr, $($field:tt)*) => {
        tracing::error!("❌ {} failed: {}", $op_name, $error);
        tracing::debug!(
            operation = $op_name,
            error = %$error,
            $($field)*,
            "Operation failed"
        );
    };
}

/// Log a warning with context
#[macro_export]
macro_rules! log_warning {
    ($msg:expr) => {
        tracing::warn!("⚠️  {}", $msg);
        tracing::debug!(warning = $msg, "Warning occurred");
    };
    ($msg:expr, $($field:tt)*) => {
        tracing::warn!("⚠️  {}", $msg);
        tracing::debug!(warning = $msg, $($field)*, "Warning occurred");
    };
}

/// Log progress information (INFO in console, DEBUG details in file)
#[macro_export]
macro_rules! log_progress {
    ($msg:expr) => {
        tracing::info!("⏳ {}", $msg);
        tracing::debug!(progress = $msg, "Progress update");
    };
    ($msg:expr, $($field:tt)*) => {
        tracing::info!("⏳ {}", $msg);
        tracing::debug!(progress = $msg, $($field)*, "Progress update");
    };
}

/// Log detailed debug information (only in file, not in console)
#[macro_export]
macro_rules! log_debug_details {
    ($($field:tt)*) => {
        tracing::debug!($($field)*);
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_macros_compile() {
        // Just verify macros compile correctly
        log_operation_start!("Test operation");
        log_operation_success!("Test operation");
        log_operation_error!("Test operation", "test error");
        log_warning!("Test warning");
        log_progress!("Test progress");
        log_debug_details!(test_field = "value", "Debug details");
    }
}
