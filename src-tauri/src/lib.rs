use std::sync::{ Arc, Mutex };
use tauri::Emitter;
use tracing::{ info, error };

#[macro_use]
mod log_utils;
mod errors;
mod paths;
mod constants;
mod models;
mod huggingface;
mod ovms;
mod chat;
mod rag;
mod mcp;
mod logging;
mod autostart;
mod tasks;

#[tauri::command]
async fn get_default_download_path() -> Result<String, String> {
    let default_path = paths::get_models_dir().map_err(|e| e.to_string())?;
    
    // Return the absolute path
    match std::fs::canonicalize(&default_path) {
        Ok(abs_path) => Ok(abs_path.to_string_lossy().to_string()),
        Err(_) => Ok(default_path.to_string_lossy().to_string()),
    }
}

#[tauri::command]
async fn get_user_profile_dir() -> Result<String, String> {
    paths::get_home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_home_dir() -> Result<String, String> {
    paths::get_home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_initialization_status() -> Result<InitializationStatus, String> {
    let status_mutex = INIT_STATUS.get_or_init(||
        Arc::new(
            Mutex::new(InitializationStatus {
                step: "not_started".to_string(),
                message: "Initialization not started".to_string(),
                progress: 0,
                is_complete: false,
                has_error: false,
                error_message: None,
            })
        )
    );

    let status = status_mutex.lock().unwrap();
    Ok(status.clone())
}

#[derive(Clone, serde::Serialize)]
struct InitializationStatus {
    step: String,
    message: String,
    progress: u8,
    is_complete: bool,
    has_error: bool,
    error_message: Option<String>,
}

// Global initialization status
static INIT_STATUS: std::sync::OnceLock<Arc<Mutex<InitializationStatus>>> = std::sync::OnceLock::new();

async fn initialize_ovms(app_handle: tauri::AppHandle) {
    log_operation_start!("OVMS initialization");
    
    let status_mutex = INIT_STATUS.get_or_init(||
        Arc::new(
            Mutex::new(InitializationStatus {
                step: "starting".to_string(),
                message: "Initializing OVMS...".to_string(),
                progress: 0,
                is_complete: false,
                has_error: false,
                error_message: None,
            })
        )
    );

    // BGE models check removed - models will be downloaded on-demand when user accesses RAG features
    // See DocumentsPage.tsx for the on-demand download implementation

    // Update status: Starting OVMS check
    {
        let mut status = status_mutex.lock().unwrap();
        status.step = "checking".to_string();
        status.message = "Checking if OVMS is present...".to_string();
        status.progress = 15;
        app_handle
            .emit("ovms-init-status", &*status)
            .unwrap_or_else(|e| {
                log_warning!("Failed to emit init status", error = %e);
            });
    }

    tracing::debug!("Checking OVMS presence");
    
    // Check if OVMS is present
    if !ovms::is_ovms_present(Some(&app_handle)) {
        log_progress!("OVMS not found, downloading...");
        
        // Update status: Downloading
        {
            let mut status = status_mutex.lock().unwrap();
            status.step = "downloading".to_string();
            status.message = "OVMS not found, downloading...".to_string();
            status.progress = 25;
            app_handle
                .emit("ovms-init-status", &*status)
                .unwrap_or_else(|e| {
                    log_warning!("Failed to emit init status", error = %e);
                });
        }

        match ovms::download_ovms(app_handle.clone()).await {
            Ok(msg) => {
                tracing::debug!(message = %msg, "OVMS download completed");

                // Update status: Downloaded
                {
                    let mut status = status_mutex.lock().unwrap();
                    status.step = "downloaded".to_string();
                    status.message = "OVMS downloaded successfully".to_string();
                    status.progress = 75;
                    app_handle
                        .emit("ovms-init-status", &*status)
                        .unwrap_or_else(|e| {
                            log_warning!("Failed to emit init status", error = %e);
                        });
                }

                // Update status: Creating config
                {
                    let mut status = status_mutex.lock().unwrap();
                    status.step = "creating_config".to_string();
                    status.message = "Creating OVMS configuration...".to_string();
                    status.progress = 77;
                    app_handle
                        .emit("ovms-init-status", &*status)
                        .unwrap_or_else(|e| {
                            log_warning!("Failed to emit init status", error = %e);
                        });
                }

                // Create initial OVMS config with BGE models
                let home_dir = match
                    std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME"))
                {
                    Ok(home) => home,
                    Err(_) => {
                        log_operation_error!("OVMS initialization", "Failed to get home directory");
                        return;
                    }
                };

                let bge_model_path =
                    format!("{}/.sparrow/models/OpenVINO/bge-base-en-v1.5-int8-ov", home_dir);

                match
                    ovms::create_ovms_config(
                        app_handle.clone(),
                        "bge-base-en-v1.5-int8-ov".to_string(),
                        bge_model_path
                    ).await
                {
                    Ok(_) => {
                        info!("OVMS config created successfully");
                    }
                    Err(e) => {
                        error!(error = %e, "Failed to create OVMS config");
                        // Continue with initialization even if config creation fails
                    }
                }
            }
            Err(e) => {
                log_operation_error!("OVMS download", &e);
                let mut status = status_mutex.lock().unwrap();
                status.has_error = true;
                status.error_message = Some(format!("Failed to download OVMS: {}", e));
                status.message = "Download failed".to_string();
                app_handle
                    .emit("ovms-init-status", &*status)
                    .unwrap_or_else(|e| {
                        log_warning!("Failed to emit init status", error = %e);
                    });
                return;
            }
        }
    } else {
        tracing::debug!("OVMS already present, skipping download");

        // Update status: Present
        {
            let mut status = status_mutex.lock().unwrap();
            status.step = "present".to_string();
            status.message = "OVMS already present".to_string();
            status.progress = 75;
            app_handle
                .emit("ovms-init-status", &*status)
                .unwrap_or_else(|e| {
                    log_warning!("Failed to emit init status", error = %e);
                });
        }

        // Check if OVMS config already exists
        let config_path = ovms::get_ovms_config_path(Some(&app_handle));
        if !config_path.exists() {
            log_progress!("Creating initial OVMS config...");

            // Update status: Creating config
            {
                let mut status = status_mutex.lock().unwrap();
                status.step = "creating_config".to_string();
                status.message = "Creating OVMS configuration...".to_string();
                status.progress = 77;
                app_handle
                    .emit("ovms-init-status", &*status)
                    .unwrap_or_else(|e| {
                        log_warning!("Failed to emit init status", error = %e);
                    });
            }

            // Create initial OVMS config with BGE models
            if let Ok(home_dir) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
                let bge_model_path =
                    format!("{}/.sparrow/models/OpenVINO/bge-base-en-v1.5-int8-ov", home_dir);

                tracing::debug!(model = "bge-base-en-v1.5-int8-ov", path = %bge_model_path, "Creating OVMS config for BGE model");
                
                match
                    ovms::create_ovms_config(
                        app_handle.clone(),
                        "bge-base-en-v1.5-int8-ov".to_string(),
                        bge_model_path
                    ).await
                {
                    Ok(_) => {
                        tracing::debug!("OVMS config created successfully");
                    }
                    Err(e) => {
                        log_warning!("Failed to create initial OVMS config", error = %e, note = "continuing initialization");
                    }
                }
            } else {
                log_warning!("Failed to get home directory for OVMS config", note = "skipping config creation");
            }
        } else {
            tracing::debug!("OVMS config already exists, skipping config creation");
        }
    }

    // Start OVMS server
    log_progress!("Starting OVMS server...");
    {
        let mut status = status_mutex.lock().unwrap();
        status.step = "starting_server".to_string();
        status.message = "Starting OVMS server...".to_string();
        status.progress = 85;
        app_handle
            .emit("ovms-init-status", &*status)
            .unwrap_or_else(|e| {
                log_warning!("Failed to emit init status", error = %e);
            });
    }

    match ovms::start_ovms_server(app_handle.clone()).await {
        Ok(msg) => {
            log_operation_success!("OVMS initialization");
            tracing::debug!(message = %msg, "OVMS server started successfully");
            let mut status = status_mutex.lock().unwrap();
            status.step = "complete".to_string();
            status.message = "OVMS initialization complete".to_string();
            status.progress = 100;
            status.is_complete = true;
            app_handle
                .emit("ovms-init-status", &*status)
                .unwrap_or_else(|e| {
                    log_warning!("Failed to emit init status", error = %e);
                });
        }
        Err(e) => {
            log_operation_error!("OVMS server startup", &e);
            let mut status = status_mutex.lock().unwrap();
            status.has_error = true;
            status.error_message = Some(format!("Failed to start OVMS server: {}", e));
            status.message = "Server startup failed".to_string();
            app_handle
                .emit("ovms-init-status", &*status)
                .unwrap_or_else(|e| {
                    log_warning!("Failed to emit init status", error = %e);
                });
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Build the Tauri log plugin with custom configuration
    let log_plugin = match logging::build_tauri_log_plugin() {
        Ok(builder) => builder.build(),
        Err(e) => {
            eprintln!("Failed to configure logging plugin: {}", e);
            // Fallback to default configuration
            tauri_plugin_log::Builder::new().build()
        }
    };

    tauri::Builder
        ::default()
        .plugin(log_plugin)
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(
            tauri::generate_handler![
                huggingface::search_models,
                huggingface::get_model_info,
                huggingface::download_entire_model,
                huggingface::check_model_update_status,
                huggingface::check_rag_models_exist,
                models::check_downloaded_models,
                models::delete_downloaded_model,
                models::open_model_folder,
                models::list_directory_names,
                models::delete_directory,
                get_default_download_path,
                get_user_profile_dir,
                get_home_dir,
                get_initialization_status,
                ovms::download_ovms,
                ovms::check_ovms_present,
                ovms::start_ovms_server,
                ovms::create_ovms_config,
                ovms::update_ovms_config,
                ovms::reload_ovms_config,
                ovms::load_model,
                ovms::unload_model,
                ovms::get_loaded_model,
                chat::chat_with_loaded_model_streaming,
                ovms::check_ovms_status,
                ovms::get_ovms_model_metadata,
                chat::get_chat_sessions,
                chat::create_chat_session,
                chat::create_temporary_chat_session,
                chat::persist_temporary_session,
                chat::add_message_to_temporary_session,
                chat::update_chat_session,
                chat::delete_chat_session,
                chat::set_active_chat_session,
                chat::add_message_to_session,
                chat::get_session_messages,
                chat::get_conversation_history,
                chat::stop_chat_streaming,
                chat::chat_with_rag_streaming,
                rag::documents::process_document,
                rag::documents::save_temp_file,
                rag::embeddings::create_document_embeddings,
                rag::embeddings::create_query_embedding,
                rag::vector_store::store_documents,
                rag::vector_store::search_documents,
                rag::vector_store::get_all_documents,
                rag::vector_store::delete_document_by_id,
                rag::vector_store::get_document_count,
                rag::vector_store::clear_all_documents,
                rag::vector_store::get_all_files,
                rag::vector_store::get_file_chunks,
                rag::vector_store::delete_file_by_path,
                rag::reranker::rerank_search_results,
                rag::reranker::rerank_search_results_simple,
                rag::search::search_documents_by_query,
                rag::search::get_search_suggestions,
                mcp::get_mcp_servers,
                mcp::add_mcp_server,
                mcp::edit_mcp_server,
                mcp::remove_mcp_server,
                mcp::connect_mcp_server,
                mcp::disconnect_mcp_server,
                mcp::get_mcp_server_info,
                mcp::fetch_mcp_server_tools,
                mcp::fetch_mcp_server_tools_details,
                mcp::get_all_mcp_tools_for_chat,
                mcp::call_mcp_tool,
                mcp::toggle_mcp_server_auto_connect,
                mcp::enable_all_auto_connect,
                mcp::auto_connect_mcp_servers,
                mcp::get_builtin_tools,
                mcp::execute_builtin_tool,
                mcp::get_all_available_tools,
                autostart::enable_autostart,
                autostart::disable_autostart,
                autostart::is_autostart_enabled,
                autostart::toggle_autostart,
                tasks::create_task,
                tasks::get_tasks,
                tasks::get_task,
                tasks::update_task,
                tasks::delete_task,
                tasks::toggle_task,
                tasks::execute_task_manually,
                tasks::get_task_logs
            ]
        )
        .setup(|app| {
            // Log startup message now that logging is configured
            tracing::info!("🚀 SparrowAI starting...");
            tracing::debug!("Tauri application setup initiated");
            
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                initialize_ovms(handle).await;
            });

            // Start periodic log cleanup task
            tauri::async_runtime::spawn(async move {
                logging::periodic_cleanup_task().await;
            });

            // Start task scheduler
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tasks::start_task_scheduler(handle).await;
            });

            Ok(())
        })

        .on_window_event(|_window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                // Stop OVMS server when app is closing
                if let Err(e) = ovms::stop_ovms_server() {
                    log_operation_error!("OVMS server shutdown", &e);
                } else {
                    log_operation_success!("OVMS server shutdown");
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
