use crate::database;
use crate::commands::system::get_system_memory;
use crate::commands::mods::get_plugin_order;
use crate::commands::collections::search_nexus_mods_cmd;
use crate::commands::collections::get_nexus_mod_detail;
use crate::commands::nexus::get_nexus_mod_files;
use crate::commands::plugins::sort_plugins_loot;
use crate::commands::diagnostics::find_crash_logs_cmd;
use crate::commands::diagnostics::analyze_crash_log_cmd;
use crate::commands::system::KNOWN_FRAMEWORKS;
use crate::crashlog;
use crate::profiles;
use crate::cleaner;
use crate::deployer;
use crate::google_oauth;
use crate::instruction_types;
use crate::llm_chat;
use crate::llm_parser;
use crate::mod_dependencies;
use crate::mod_recommendations;
use crate::preflight;
use crate::wine_compat;
use crate::{AppState, resolve_bottle, resolve_game};
use std::path::PathBuf;
use tauri::Emitter;
use tauri::State;

// --- LLM Chat Commands ---

/// Get the current chat session state.
#[tauri::command]
pub async fn chat_get_state(state: State<'_, AppState>) -> Result<llm_chat::ChatState, String> {
    let session = state.chat_session.lock().await;
    let ollama_status = llm_parser::check_ollama_status().await;
    let cloud_provider = match &session.backend {
        llm_chat::LlmBackend::Cloud { ref provider, .. } => Some(provider.clone()),
        llm_chat::LlmBackend::GeminiOAuth => Some("gemini_oauth".to_string()),
        _ => None,
    };
    let google_auth = Some(google_oauth::get_google_auth_status());
    Ok(llm_chat::ChatState {
        model: session.model.clone(),
        backend: session.backend.clone(),
        loaded: session.model.is_some(),
        messages: session.messages.clone(),
        available_models: ollama_status
            .available_models
            .into_iter()
            .map(|m| instruction_types::OllamaModel {
                name: m.name,
                size_bytes: m.size_bytes,
                size_display: m.size_display,
                description: m.description,
                expected_accuracy: m.expected_accuracy,
                supports_tool_use: m.supports_tool_use,
                min_memory_bytes: 0,
            })
            .collect(),
        cloud_provider,
        google_auth,
    })
}

/// Resolve game display name from game_id.
pub fn game_display_name(game_id: &str) -> &str {
    match game_id {
        "skyrimse" => "Skyrim Special Edition",
        "skyrim" => "Skyrim",
        "fallout4" => "Fallout 4",
        "fallout3" => "Fallout 3",
        "falloutnv" => "Fallout: New Vegas",
        "oblivion" => "The Elder Scrolls IV: Oblivion",
        "morrowind" => "Morrowind",
        "starfield" => "Starfield",
        "hogwartslegacy" => "Hogwarts Legacy",
        other => other,
    }
}

/// Load a model for chat and initialize the session.
#[tauri::command]
pub async fn chat_load_model(
    model_name: String,
    game_id: String,
    bottle_name: String,
    current_page: Option<String>,
    backend: Option<String>,
    cloud_provider: Option<String>,
    cloud_api_key: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let backend_enum = match backend.as_deref() {
        Some("cloud") => {
            let provider = cloud_provider.ok_or("cloud_provider is required for cloud backend")?;
            let api_key = cloud_api_key.ok_or("cloud_api_key is required for cloud backend")?;
            llm_chat::LlmBackend::Cloud { provider, api_key }
        }
        Some("gemini_oauth") => llm_chat::LlmBackend::GeminiOAuth,
        Some("mlx") => llm_chat::LlmBackend::Mlx,
        _ => llm_chat::LlmBackend::Ollama,
    };

    // Resolve model name for the backend
    let resolved_model = match &backend_enum {
        llm_chat::LlmBackend::Mlx => llm_chat::mlx_model_name(&model_name),
        llm_chat::LlmBackend::Ollama => model_name.clone(),
        llm_chat::LlmBackend::Cloud { ref provider, .. } => llm_chat::cloud_model_display(provider),
        llm_chat::LlmBackend::GeminiOAuth => llm_chat::cloud_model_display("gemini_oauth"),
    };

    // Start the MLX server if needed
    if backend_enum == llm_chat::LlmBackend::Mlx {
        llm_chat::start_mlx_server(&resolved_model).await?;
    }

    // Compute context config based on system memory
    let mem_bytes = get_system_memory().unwrap_or(16_000_000_000);
    let num_ctx = instruction_types::context_size_for_memory(mem_bytes);

    // Load the model (no-op for cloud)
    llm_chat::load_model(&backend_enum, &resolved_model, num_ctx).await?;

    let db = state.db.clone();
    let gid = game_id.clone();
    let bn = bottle_name.clone();
    let (mod_count, wine_warnings_text) = tokio::task::spawn_blocking(move || {
        let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
        let mod_count = mods.len();

        // Run Wine compat check on all installed mods
        let compat_input = wine_compat::build_compat_input(&mods);
        let warnings = wine_compat::check_all_mods_wine_compat(&compat_input);
        let warnings_text = if warnings.is_empty() {
            None
        } else {
            Some(wine_compat::format_warnings_report(&warnings))
        };
        (mod_count, warnings_text)
    })
    .await
    .unwrap_or((0, None));

    let game_name = game_display_name(&game_id);
    let page = current_page.as_deref().unwrap_or("Mods");

    let mut session = state.chat_session.lock().await;
    session.model = Some(resolved_model.clone());
    session.backend = backend_enum;
    session.messages.clear();
    session.touch();

    // Add system message
    let system = llm_chat::build_chat_system_prompt(
        game_name,
        mod_count,
        "Wine/CrossOver",
        page,
        None,
        wine_warnings_text.as_deref(),
        &session.backend,
    );
    session.messages.push(llm_chat::ChatMessage {
        role: "system".into(),
        content: system,
        tool_calls: None,
        mentioned_mods: None,
        timestamp: None,
    });

    // Restore recent chat history from DB
    {
        let db = state.db.clone();
        let gid = game_id.clone();
        let bn = bottle_name.clone();
        let history = tokio::task::spawn_blocking(move || {
            db.load_chat_history(&gid, &bn, 50).unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        if !history.is_empty() {
            log::info!("[CHAT] Restored {} messages from history", history.len());
            session.messages.extend(history);
        }
    }

    Ok(())
}

/// Unload the chat model and clear the session.
#[tauri::command]
pub async fn chat_unload_model(state: State<'_, AppState>) -> Result<(), String> {
    let mut session = state.chat_session.lock().await;
    if let Some(ref model) = session.model {
        let _ = llm_chat::unload_model(&session.backend, model).await;
    }
    session.model = None;
    session.messages.clear();
    Ok(())
}

/// Check which MLX models are already cached locally in ~/.cache/huggingface/hub/.
#[tauri::command]
pub async fn get_cached_mlx_models() -> Vec<String> {
    let mut cached = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let hub_dir = home.join(".cache/huggingface/hub");
        if let Ok(entries) = std::fs::read_dir(&hub_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("models--") && entry.path().is_dir() {
                    // Convert "models--org--name" back to "org/name"
                    let model_id = name
                        .strip_prefix("models--")
                        .unwrap_or(&name)
                        .replace("--", "/");
                    cached.push(model_id);
                }
            }
        }
    }
    cached
}

/// Delete a model from disk (Ollama: API delete, MLX: remove HuggingFace cache).
#[tauri::command]
pub async fn delete_model(model_name: String, backend: Option<String>) -> Result<String, String> {
    match backend.as_deref() {
        Some("mlx") => {
            // MLX models cached in ~/.cache/huggingface/hub/models--<org>--<name>
            let sanitized = model_name.replace("/", "--");
            if let Some(home) = dirs::home_dir() {
                let cache_dir = home
                    .join(".cache/huggingface/hub")
                    .join(format!("models--{sanitized}"));
                if cache_dir.exists() {
                    tokio::fs::remove_dir_all(&cache_dir)
                        .await
                        .map_err(|e| format!("Failed to delete: {e}"))?;
                    Ok(format!("Deleted {model_name} from MLX cache."))
                } else {
                    Err(format!("Cache directory not found for {model_name}"))
                }
            } else {
                Err("Cannot determine home directory.".into())
            }
        }
        _ => {
            // Ollama: DELETE /api/delete
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| e.to_string())?;
            let resp = client
                .delete("http://localhost:11434/api/delete")
                .json(&serde_json::json!({ "name": model_name }))
                .send()
                .await
                .map_err(|e| format!("Failed to delete model: {e}"))?;
            if resp.status().is_success() {
                Ok(format!("Deleted {model_name} from Ollama."))
            } else {
                let text = resp.text().await.unwrap_or_default();
                Err(format!("Ollama delete failed: {text}"))
            }
        }
    }
}

/// Send a user message and get the assistant response.
/// Handles tool calls automatically (one round).
#[tauri::command]
pub async fn chat_send_message(
    message: String,
    game_id: String,
    bottle_name: String,
    current_page: Option<String>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<llm_chat::ChatResponse, String> {
    let (model, backend, tier, messages) = {
        let mut session = state.chat_session.lock().await;
        session.touch();
        let model = session.model.clone().ok_or("No model loaded")?;
        let backend = session.backend.clone();
        let tier = instruction_types::ModelCapabilityTier::from_model_name(&model);

        // Update system prompt with current page context if it changed
        if let Some(ref page) = current_page {
            let backend_ref = session.backend.clone();
            if let Some(system_msg) = session.messages.first_mut() {
                if system_msg.role == "system"
                    && !system_msg.content.contains(&format!("Page: {page}"))
                {
                    // Rebuild system prompt with updated page + wine compat
                    let db = state.db.clone();
                    let gid = game_id.clone();
                    let bn = bottle_name.clone();
                    let mods_list = db.list_mods_summary(&gid, &bn).unwrap_or_default();
                    let mod_count = mods_list.len();
                    let compat_input = wine_compat::build_compat_input(&mods_list);
                    let warnings = wine_compat::check_all_mods_wine_compat(&compat_input);
                    let warnings_text = if warnings.is_empty() {
                        None
                    } else {
                        Some(wine_compat::format_warnings_report(&warnings))
                    };
                    system_msg.content = llm_chat::build_chat_system_prompt(
                        game_display_name(&game_id),
                        mod_count,
                        "Wine/CrossOver",
                        page,
                        None,
                        warnings_text.as_deref(),
                        &backend_ref,
                    );
                }
            }
        }

        // Add user message
        let user_msg = llm_chat::ChatMessage {
            role: "user".into(),
            content: message,
            tool_calls: None,
            mentioned_mods: None,
            timestamp: None,
        };
        session.messages.push(user_msg.clone());

        // Persist user message to DB
        let db = state.db.clone();
        let gid = game_id.clone();
        let bn = bottle_name.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Err(e) = db.save_chat_message(&gid, &bn, &user_msg) {
                log::warn!("[CHAT] Failed to save user message: {e}");
            }
        });

        (model, backend, tier, session.messages.clone())
    };

    let tools = llm_chat::get_chat_tools(tier);

    // Compute context config based on system memory
    let mem_bytes = get_system_memory().unwrap_or(16_000_000_000);
    let num_ctx = instruction_types::context_size_for_memory(mem_bytes);
    let max_tokens = instruction_types::max_response_tokens(num_ctx);

    // Get LLM response with streaming
    log::info!("[CHAT] Sending to {:?} model={}", backend, model);
    let handle = app_handle.clone();
    let response = llm_chat::chat_send_streaming(
        &backend,
        &model,
        &messages,
        &tools,
        num_ctx,
        max_tokens,
        move |token, phase| {
            let _ = handle.emit(
                "chat-stream-token",
                serde_json::json!({
                    "text": token,
                    "phase": phase,
                }),
            );
        },
    )
    .await?;

    log::info!(
        "[CHAT] Response: content_len={} tool_calls={:?}",
        response.content.len(),
        response.tool_calls.as_ref().map(|tc| tc.len())
    );

    let mut all_tool_results = Vec::new();
    let mut current_response = response;
    let max_tool_rounds = 5;

    for round in 0..max_tool_rounds {
        // Check if this response has tool calls
        let has_tool_calls = current_response
            .tool_calls
            .as_ref()
            .map(|tc| !tc.is_empty())
            .unwrap_or(false);
        if !has_tool_calls {
            break;
        }

        let mut round_results = Vec::new();
        for tc in current_response.tool_calls.as_ref().unwrap() {
            log::info!(
                "[CHAT] [round {}] Executing tool: {} args={}",
                round,
                tc.function.name,
                tc.function.arguments
            );
            let display = tool_display_name(&tc.function.name, &tc.function.arguments);
            let _ = app_handle.emit(
                "chat-tool-status",
                serde_json::json!({
                    "tool_name": tc.function.name,
                    "status": "running",
                    "display_text": display,
                }),
            );

            // UI control tools are handled here (need app_handle for event emission)
            let result = if tc.function.name == "navigate_ui" {
                let page = tc
                    .function
                    .arguments
                    .get("page")
                    .and_then(|p| p.as_str())
                    .unwrap_or("mods");
                let _ = app_handle.emit("chat-navigate", serde_json::json!({ "page": page }));
                llm_chat::ToolResult {
                    tool_name: "navigate_ui".into(),
                    result: format!("Navigated to {} page.", page),
                    success: true,
                    display_name: format!("Navigating to {}", page),
                    structured_data: None,
                }
            } else if tc.function.name == "open_nexus_mod" {
                let mod_id = tc
                    .function
                    .arguments
                    .get("mod_id")
                    .and_then(|i| i.as_i64())
                    .unwrap_or(0);
                let mod_name = tc
                    .function
                    .arguments
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("mod");
                let _ = app_handle.emit(
                    "chat-open-nexus-mod",
                    serde_json::json!({
                        "mod_id": mod_id,
                        "name": mod_name,
                    }),
                );
                llm_chat::ToolResult {
                    tool_name: "open_nexus_mod".into(),
                    result: format!("Opened {} (ID: {}) in Corkscrew's Discover tab with images and install button.", mod_name, mod_id),
                    success: true,
                    display_name: format!("Opening {} in Discover", mod_name),
                    structured_data: None,
                }
            } else {
                execute_tool(
                    &tc.function.name,
                    &tc.function.arguments,
                    &game_id,
                    &bottle_name,
                    &state,
                )
                .await
            };
            log::info!(
                "[CHAT] [round {}] Tool result: success={} len={}",
                round,
                result.success,
                result.result.len()
            );

            let _ = app_handle.emit(
                "chat-tool-status",
                serde_json::json!({
                    "tool_name": tc.function.name,
                    "status": "complete",
                    "display_text": display,
                }),
            );

            round_results.push(result);
        }

        // Store assistant response + tool results in session
        {
            let mut session = state.chat_session.lock().await;
            session.messages.push(current_response.clone());
            for tr in &round_results {
                session.messages.push(llm_chat::ChatMessage {
                    role: "tool".into(),
                    content: tr.result.clone(),
                    tool_calls: None,
                    mentioned_mods: None,
                    timestamp: None,
                });
            }
        }
        all_tool_results.extend(round_results);

        // Get follow-up response WITH tools so model can chain calls
        log::info!("[CHAT] Making follow-up call (round {})", round);
        let messages = {
            let session = state.chat_session.lock().await;
            log::info!(
                "[CHAT] Session has {} messages for follow-up",
                session.messages.len()
            );
            session.messages.clone()
        };
        let handle2 = app_handle.clone();
        let followup = llm_chat::chat_send_streaming(
            &backend,
            &model,
            &messages,
            &tools,
            num_ctx,
            max_tokens,
            move |token, phase| {
                let _ = handle2.emit(
                    "chat-stream-token",
                    serde_json::json!({
                        "text": token,
                        "phase": phase,
                    }),
                );
            },
        )
        .await?;
        log::info!(
            "[CHAT] Follow-up response (round {}): content_len={} tool_calls={:?}",
            round,
            followup.content.len(),
            followup.tool_calls.as_ref().map(|tc| tc.len())
        );
        current_response = followup;
    }

    // If we exhausted tool rounds and still have no content, force a text-only response
    if !all_tool_results.is_empty() && current_response.content.trim().is_empty() {
        log::info!(
            "[CHAT] Forcing text-only follow-up (no content after {} tool rounds)",
            max_tool_rounds
        );
        // Store the last tool-call response + its results
        {
            let mut session = state.chat_session.lock().await;
            session.messages.push(current_response.clone());
            // If there are pending tool calls, add a synthetic tool result
            if let Some(ref tcs) = current_response.tool_calls {
                for _tc in tcs {
                    session.messages.push(llm_chat::ChatMessage {
                        role: "tool".into(),
                        content: "Tool call limit reached. Please summarize what you found so far."
                            .into(),
                        tool_calls: None,
                        mentioned_mods: None,
                        timestamp: None,
                    });
                }
            }
        }
        let messages = {
            let session = state.chat_session.lock().await;
            session.messages.clone()
        };
        let handle3 = app_handle.clone();
        let forced = llm_chat::chat_send_streaming(
            &backend,
            &model,
            &messages,
            &[],
            num_ctx,
            max_tokens,
            move |token, phase| {
                let _ = handle3.emit(
                    "chat-stream-token",
                    serde_json::json!({
                        "text": token,
                        "phase": phase,
                    }),
                );
            },
        )
        .await?;
        log::info!(
            "[CHAT] Forced text response: content_len={}",
            forced.content.len()
        );
        current_response = forced;
    }

    let tool_results = all_tool_results;

    // Scan for mentioned mods and attach to the final response
    let mut final_msg = current_response;
    let mentioned = scan_mentioned_mods(
        &final_msg.content,
        &tool_results,
        &game_id,
        &bottle_name,
        &state,
    )
    .await;
    if !mentioned.is_empty() {
        final_msg.mentioned_mods = Some(mentioned);
    }

    // Store final response (with mentioned_mods) in session
    {
        let mut session = state.chat_session.lock().await;
        session.messages.push(final_msg.clone());
    }

    // Persist assistant message to DB (only if it has content)
    if !final_msg.content.trim().is_empty() {
        let db = state.db.clone();
        let gid = game_id.clone();
        let bn = bottle_name.clone();
        let msg_to_save = final_msg.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Err(e) = db.save_chat_message(&gid, &bn, &msg_to_save) {
                log::warn!("[CHAT] Failed to save assistant message: {e}");
            }
        });
    }

    Ok(llm_chat::ChatResponse {
        message: final_msg,
        tool_results,
        needs_confirmation: false,
        pending_tool_calls: None,
    })
}

/// Clear chat history but keep model loaded.
#[tauri::command]
pub async fn chat_clear_history(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut session = state.chat_session.lock().await;
    // Keep system message, clear the rest
    if let Some(system) = session.messages.first().cloned() {
        session.messages.clear();
        if system.role == "system" {
            session.messages.push(system);
        }
    }

    // Also clear persisted history
    let db = state.db.clone();
    let _ = tokio::task::spawn_blocking(move || {
        if let Err(e) = db.clear_chat_history(&game_id, &bottle_name) {
            log::warn!("[CHAT] Failed to clear DB history: {e}");
        }
    });

    Ok(())
}

/// Get persisted chat history for display before model is loaded.
#[tauri::command]
pub async fn chat_get_history(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<llm_chat::ChatMessage>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.load_chat_history(&game_id, &bottle_name, 50)
            .map_err(|e| format!("Failed to load chat history: {e}"))
    })
    .await
    .map_err(|e| format!("Task join error: {e}"))?
}

/// Validate a cloud API key by making a minimal request.
#[tauri::command]
pub async fn chat_validate_cloud_key(
    provider: String,
    api_key: String,
) -> Result<String, String> {
    llm_chat::validate_cloud_key(&provider, &api_key).await
}

/// Get contextual conversation starters based on game state.
#[tauri::command]
pub async fn chat_get_starters(
    game_id: String,
    bottle_name: String,
    current_page: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<llm_chat::ChatStarter>, String> {
    let db = state.db.clone();
    let gid = game_id.clone();
    let bn = bottle_name.clone();

    let (mod_count, _enabled_count, disabled_count, has_conflicts) =
        tokio::task::spawn_blocking(move || {
            let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
            let enabled = mods.iter().filter(|m| m.enabled).count();
            let disabled = mods.len() - enabled;
            let conflicts = db
                .find_all_conflicts(&gid, &bn)
                .map(|c| !c.is_empty())
                .unwrap_or(false);
            (mods.len(), enabled, disabled, conflicts)
        })
        .await
        .unwrap_or((0, 0, 0, false));

    // Check for new crash logs (fast filesystem stat).
    let crash_gid = game_id.clone();
    let crash_bn = bottle_name.clone();
    let crash_info = tokio::task::spawn_blocking(move || {
        match resolve_bottle(&crash_bn) {
            Ok(bottle) => {
                crashlog::check_new_crashes(&PathBuf::from(&bottle.path), &crash_gid)
            }
            Err(_) => crashlog::NewCrashInfo {
                count: 0,
                entries: vec![],
            },
        }
    })
    .await
    .unwrap_or(crashlog::NewCrashInfo {
        count: 0,
        entries: vec![],
    });

    let mut starters = Vec::new();

    // Prepend crash starter if new crashes detected.
    if crash_info.count > 0 {
        let label = if crash_info.count == 1 {
            "\u{1F534} New crash detected".to_string()
        } else {
            format!("\u{1F534} {} new crashes detected", crash_info.count)
        };
        starters.push(llm_chat::ChatStarter {
            label,
            prompt:
                "I just crashed. Can you analyze my latest crash log and tell me what went wrong?"
                    .into(),
        });
    }

    // Quick health check (wine compat only — fast, no expensive dep/conflict queries)
    let health_db = state.db.clone();
    let health_gid = game_id.clone();
    let health_bn = bottle_name.clone();
    let wine_warning_count = tokio::task::spawn_blocking(move || {
        let mods = health_db
            .list_mods_summary(&health_gid, &health_bn)
            .unwrap_or_default();
        let compat_input = wine_compat::build_compat_input(&mods);
        let warnings = wine_compat::check_all_mods_wine_compat(&compat_input);
        warnings
            .iter()
            .filter(|(_, w)| matches!(w.severity, wine_compat::Severity::Crash | wine_compat::Severity::Broken))
            .count()
    })
    .await
    .unwrap_or(0);

    if wine_warning_count > 0 {
        starters.push(llm_chat::ChatStarter {
            label: format!(
                "\u{26A0}\u{FE0F} {} Wine-incompatible mod{}",
                wine_warning_count,
                if wine_warning_count == 1 { "" } else { "s" }
            ),
            prompt: "Check my mod health score and tell me what issues to fix.".into(),
        });
    }

    let page = current_page.as_deref().unwrap_or("Mods");

    if has_conflicts {
        starters.push(llm_chat::ChatStarter {
            label: "Explain my mod conflicts".into(),
            prompt: "Check my mod conflicts and explain what's happening. Are any of them serious?"
                .into(),
        });
    }

    if disabled_count > 5 {
        starters.push(llm_chat::ChatStarter {
            label: format!("{} mods are disabled", disabled_count),
            prompt: "I have a lot of disabled mods. Can you review them and tell me which ones I might want to enable?".into(),
        });
    }

    match page {
        "Load Order" => {
            starters.push(llm_chat::ChatStarter {
                label: "Check my load order".into(),
                prompt: "Is my load order correct? Are there any issues I should fix?".into(),
            });
        }
        "Crash Logs" => {
            starters.push(llm_chat::ChatStarter {
                label: "Analyze my latest crash".into(),
                prompt: "Check my crash logs and analyze the most recent crash.".into(),
            });
        }
        "Discover" => {
            starters.push(llm_chat::ChatStarter {
                label: "Recommend mods for me".into(),
                prompt: "Based on my installed mods, what would you recommend I add?".into(),
            });
        }
        _ => {}
    }

    if mod_count > 0 {
        starters.push(llm_chat::ChatStarter {
            label: format!("Overview of my {} mods", mod_count),
            prompt: "Give me an overview of my mod setup. How many mods do I have, any issues?"
                .into(),
        });
    }

    if starters.len() < 3 {
        starters.push(llm_chat::ChatStarter {
            label: "Find me a mod".into(),
            prompt: "Help me find a good mod. What kind of mods are you looking for?".into(),
        });
    }

    starters.truncate(4);
    Ok(starters)
}

/// Helper: fuzzy-find a mod by name from the summary list.
pub fn find_mod_by_name<'a>(
    mods: &'a [database::ModSummary],
    name: &str,
) -> Option<&'a database::ModSummary> {
    let lower = name.to_lowercase();
    mods.iter()
        .find(|m| m.name.to_lowercase() == lower)
        .or_else(|| {
            let matches: Vec<_> = mods
                .iter()
                .filter(|m| m.name.to_lowercase().contains(&lower))
                .collect();
            if matches.len() == 1 {
                Some(matches[0])
            } else {
                None
            }
        })
}

/// Web search via DuckDuckGo lite HTML (no API key needed).
pub async fn web_search_ddg(query: &str) -> String {
    let client = match reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => return format!("Search failed: {e}"),
    };

    // Use Brave Search (DuckDuckGo blocks programmatic access with CAPTCHAs)
    let resp = match client
        .get("https://search.brave.com/search")
        .query(&[("q", query), ("source", "web")])
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return format!("Search failed: {e}"),
    };
    let html = match resp.text().await {
        Ok(t) => t,
        Err(e) => return format!("Search failed: {e}"),
    };

    // Brave Search: results have <a> tags with snippet-title spans
    // Structure: <div class="snippet"> containing <a href="URL"> with nested <span class="snippet-title">Title</span>
    // and <p class="snippet-description">Description</p>
    let mut results = Vec::new();

    pub fn strip_tags(s: &str) -> String {
        let mut clean = String::new();
        let mut in_tag = false;
        for ch in s.chars() {
            match ch {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => clean.push(ch),
                _ => {}
            }
        }
        clean.trim().to_string()
    }

    // Split by snippet blocks
    let parts: Vec<&str> = html.split("class=\"snippet ").collect();
    for part in parts.iter().skip(1) {
        if results.len() >= 6 {
            break;
        }

        // Extract URL from first href="https://..."
        let url = part
            .split("href=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("");
        if url.is_empty() || !url.starts_with("http") {
            continue;
        }

        // Extract title from snippet-title span
        let title = if let Some(pos) = part.find("snippet-title") {
            let after = &part[pos..];
            after
                .split('>')
                .nth(1)
                .and_then(|s| s.split("</span>").next())
                .map(|s| strip_tags(s))
                .unwrap_or_default()
        } else {
            String::new()
        };

        // Extract description from snippet-description
        let desc = if let Some(pos) = part.find("snippet-description") {
            let after = &part[pos..];
            after
                .split('>')
                .nth(1)
                .and_then(|s| s.split("</p>").next().or_else(|| s.split("</div>").next()))
                .map(|s| strip_tags(s))
                .unwrap_or_default()
        } else {
            String::new()
        };

        if !title.is_empty() {
            let snippet = if desc.len() > 200 {
                format!("{}...", &desc[..200])
            } else {
                desc
            };
            if snippet.is_empty() {
                results.push(format!("• {} ({})", title, url));
            } else {
                results.push(format!("• {} ({})\n  {}", title, url, snippet));
            }
        }
    }

    if results.is_empty() {
        format!("No web results found for \"{}\".", query)
    } else {
        format!("Web results for \"{}\":\n{}", query, results.join("\n"))
    }
}

/// Human-friendly display name for a tool while it's running.
pub fn tool_display_name(tool_name: &str, args: &serde_json::Value) -> String {
    match tool_name {
        "search_nexus" => {
            let q = args.get("query").and_then(|q| q.as_str()).unwrap_or("mods");
            format!("Searching NexusMods for \"{}\"...", q)
        }
        "web_search" => {
            let q = args.get("query").and_then(|q| q.as_str()).unwrap_or("...");
            format!("Searching the web for \"{}\"...", q)
        }
        "list_mods" => "Listing installed mods...".into(),
        "get_load_order" => "Checking load order...".into(),
        "get_conflicts" => "Checking mod conflicts...".into(),
        "get_mod_info" => {
            let n = args
                .get("mod_name")
                .and_then(|n| n.as_str())
                .unwrap_or("mod");
            format!("Getting info for \"{}\"...", n)
        }
        "get_nexus_mod_detail" => "Fetching mod details from NexusMods...".into(),
        "get_nexus_mod_files" => "Fetching available files...".into(),
        "download_and_install_mod" => "Downloading and installing mod...".into(),
        "sort_load_order" => "Sorting load order...".into(),
        "get_crash_logs" => "Checking crash logs...".into(),
        "analyze_crash_log" => "Analyzing crash log...".into(),
        "check_wine_compatibility" => "Checking Wine mod compatibility...".into(),
        "enable_mod" | "disable_mod" => {
            let n = args
                .get("mod_name")
                .and_then(|n| n.as_str())
                .unwrap_or("mod");
            format!(
                "{} \"{}\"...",
                if tool_name == "enable_mod" {
                    "Enabling"
                } else {
                    "Disabling"
                },
                n
            )
        }
        "check_mod_updates" => "Checking for mod updates...".into(),
        "run_preflight_check" => "Running preflight check...".into(),
        "redeploy_mods" => "Redeploying mods...".into(),
        "navigate_ui" => {
            let page = args.get("page").and_then(|p| p.as_str()).unwrap_or("page");
            format!("Navigating to {}...", page)
        }
        "open_nexus_mod" => {
            let name = args.get("name").and_then(|n| n.as_str()).unwrap_or("mod");
            format!("Opening {} in Discover...", name)
        }
        "find_needed_patches" => "Analyzing mod list for needed patches...".into(),
        "run_full_diagnostic" => "Running full diagnostic...".into(),
        "get_mod_requirements" => "Checking mod requirements...".into(),
        "batch_mod_operation" => {
            let action = args.get("action").and_then(|a| a.as_str()).unwrap_or("toggle");
            let filter = args.get("filter_value").and_then(|f| f.as_str()).unwrap_or("mods");
            format!("{}ing {} mods...", if action == "enable" { "Enabl" } else { "Disabl" }, filter)
        }
        "get_mod_health" => "Calculating mod health score...".into(),
        other => format!("Running {}...", other),
    }
}

/// Tool result display name for collapsible headers.
pub fn tool_result_display_name(tool_name: &str, result: &str) -> String {
    match tool_name {
        "list_mods" => {
            let count = result.split('\n').next().unwrap_or("").to_string();
            format!("Listed mods ({})", count.split(' ').next().unwrap_or("?"))
        }
        "search_nexus" => {
            let count = result.split('\n').next().unwrap_or("").to_string();
            format!(
                "NexusMods search ({})",
                count.split(' ').next().unwrap_or("?")
            )
        }
        "web_search" => "Web search results".into(),
        "get_load_order" => "Load order".into(),
        "get_conflicts" => "File conflicts".into(),
        "get_mod_info" => "Mod details".into(),
        "get_nexus_mod_detail" => "NexusMods details".into(),
        "get_nexus_mod_files" => "Available files".into(),
        "get_crash_logs" => "Crash logs".into(),
        "analyze_crash_log" => "Crash analysis".into(),
        "enable_mod" | "disable_mod" => result.lines().next().unwrap_or("Done").to_string(),
        "check_wine_compatibility" => "Wine compatibility check".into(),
        "find_needed_patches" => "Patch analysis".into(),
        "run_full_diagnostic" => "Diagnostic report".into(),
        "get_mod_requirements" => "Dependency check".into(),
        "batch_mod_operation" => "Batch mod operation".into(),
        "get_mod_health" => "Health score".into(),
        other => format!("{} result", other),
    }
}

/// Scan assistant response and tool results for mod references, cross-reference with installed mods.
pub async fn scan_mentioned_mods(
    content: &str,
    tool_results: &[llm_chat::ToolResult],
    game_id: &str,
    bottle_name: &str,
    state: &State<'_, AppState>,
) -> Vec<llm_chat::MentionedMod> {
    let db = state.db.clone();
    let gid = game_id.to_string();
    let bn = bottle_name.to_string();

    let mods = match tokio::task::spawn_blocking(move || {
        db.list_mods_summary(&gid, &bn).unwrap_or_default()
    })
    .await
    {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    let mut mentioned = Vec::new();
    let content_lower = content.to_lowercase();

    // Check installed mods against response text
    for m in &mods {
        let name_lower = m.name.to_lowercase();
        if name_lower.len() > 3 && content_lower.contains(&name_lower) {
            mentioned.push(llm_chat::MentionedMod {
                name: m.name.clone(),
                local_id: Some(m.id),
                nexus_mod_id: m.nexus_mod_id,
                enabled: Some(m.enabled),
                installed: true,
                picture_url: None,
            });
        }
    }

    // Check tool results for Nexus mod IDs (from structured_data)
    for tr in tool_results {
        if let Some(ref data) = tr.structured_data {
            if let Some(mods_arr) = data.as_array() {
                for nexus_mod in mods_arr {
                    let name = nexus_mod.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let mod_id = nexus_mod.get("mod_id").and_then(|i| i.as_i64());
                    let pic = nexus_mod
                        .get("picture_url")
                        .and_then(|p| p.as_str())
                        .map(String::from);
                    if !name.is_empty()
                        && !mentioned.iter().any(|m| m.name.eq_ignore_ascii_case(name))
                    {
                        let is_installed = mods.iter().any(|m| m.name.eq_ignore_ascii_case(name));
                        let local = mods.iter().find(|m| m.name.eq_ignore_ascii_case(name));
                        mentioned.push(llm_chat::MentionedMod {
                            name: name.to_string(),
                            local_id: local.map(|m| m.id),
                            nexus_mod_id: mod_id,
                            enabled: local.map(|m| m.enabled),
                            installed: is_installed,
                            picture_url: pic,
                        });
                    }
                }
            }
        }
    }

    // Limit to avoid overwhelming the UI
    mentioned.truncate(10);
    mentioned
}

/// Execute a tool call from the LLM.
pub async fn execute_tool(
    name: &str,
    args: &serde_json::Value,
    game_id: &str,
    bottle_name: &str,
    state: &State<'_, AppState>,
) -> llm_chat::ToolResult {
    let db = state.db.clone();
    let gid = game_id.to_string();
    let bn = bottle_name.to_string();

    let mut structured_data: Option<serde_json::Value> = None;

    let (result, success) = match name {
        // ── Basic: mod list & toggle ─────────────────────────────────
        "list_mods" => {
            let filter = args
                .get("filter")
                .and_then(|f| f.as_str())
                .unwrap_or("")
                .to_lowercase();
            let r = tokio::task::spawn_blocking(move || {
                let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
                let filtered: Vec<_> = if filter.is_empty() {
                    mods.iter().collect()
                } else {
                    mods.iter()
                        .filter(|m| m.name.to_lowercase().contains(&filter))
                        .collect()
                };
                let lines: Vec<String> = filtered
                    .iter()
                    .map(|m| {
                        format!(
                            "{} [{}]",
                            m.name,
                            if m.enabled { "enabled" } else { "disabled" }
                        )
                    })
                    .collect();
                format!("{} mods found:\n{}", filtered.len(), lines.join("\n"))
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "enable_mod" | "disable_mod" => {
            let mod_name = args
                .get("mod_name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let enable = name == "enable_mod";
            let r = tokio::task::spawn_blocking(move || {
                let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
                match find_mod_by_name(&mods, &mod_name) {
                    Some(m) => match db.set_enabled(m.id, enable) {
                        Ok(_) => format!(
                            "{} \"{}\"",
                            if enable { "Enabled" } else { "Disabled" },
                            m.name
                        ),
                        Err(e) => format!("Failed: {e}"),
                    },
                    None => format!("Mod \"{}\" not found", mod_name),
                }
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "get_mod_info" => {
            let mod_name = args
                .get("mod_name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let r = tokio::task::spawn_blocking(move || {
                let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
                match find_mod_by_name(&mods, &mod_name) {
                    Some(m) => format!(
                        "Name: {}\nEnabled: {}\nVersion: {}\nFiles: {}\nCategory: {}\nCollection: {}\nOptional: {}\nNexus ID: {}",
                        m.name, m.enabled, m.version, m.file_count,
                        m.auto_category.as_deref().unwrap_or("uncategorized"),
                        m.collection_name.as_deref().unwrap_or("none"),
                        m.collection_optional,
                        m.nexus_mod_id.map_or("none".to_string(), |id| id.to_string()),
                    ),
                    None => format!("Mod \"{}\" not found", mod_name),
                }
            }).await.unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "get_deployment_status" => {
            let r = tokio::task::spawn_blocking(move || {
                let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
                let enabled = mods.iter().filter(|m| m.enabled).count();
                let collections: std::collections::HashSet<_> = mods
                    .iter()
                    .filter_map(|m| m.collection_name.as_deref())
                    .collect();
                format!(
                    "{} enabled / {} total mods, {} collections",
                    enabled,
                    mods.len(),
                    collections.len()
                )
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "check_wine_compatibility" => {
            let r = tokio::task::spawn_blocking(move || {
                let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
                let compat_input = wine_compat::build_compat_input(&mods);
                let warnings = wine_compat::check_all_mods_wine_compat(&compat_input);
                if warnings.is_empty() {
                    "No Wine compatibility issues detected. All enabled mods appear compatible.".to_string()
                } else {
                    wine_compat::format_warnings_report(&warnings)
                }
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        // ── Standard: load order, conflicts, Nexus search ────────────
        "get_load_order" => {
            let r = match get_plugin_order(gid.clone(), bn.clone()).await {
                Ok(plugins) => {
                    let lines: Vec<String> = plugins
                        .iter()
                        .enumerate()
                        .map(|(i, p)| {
                            format!(
                                "{:3}. {} [{}]",
                                i,
                                p.filename,
                                if p.enabled { "active" } else { "inactive" }
                            )
                        })
                        .collect();
                    format!("{} plugins:\n{}", plugins.len(), lines.join("\n"))
                }
                Err(e) => format!("Error: {e}"),
            };
            (r, true)
        }

        "get_conflicts" => {
            let db2 = state.db.clone();
            let db3 = state.db.clone();
            let gid2 = gid.clone();
            let gid3 = gid.clone();
            let bn2 = bn.clone();
            let bn3 = bn.clone();
            let r = tokio::task::spawn_blocking(move || {
                let conflicts = match db2
                    .find_all_conflicts(&gid2, &bn2)
                    .map_err(|e| e.to_string())
                {
                    Ok(c) => c,
                    Err(e) => return format!("Error: {e}"),
                };

                if conflicts.is_empty() {
                    return "No file conflicts detected.".into();
                }

                // Build mod_name -> auto_category lookup
                let mod_categories: std::collections::HashMap<String, String> = db3
                    .list_mods_summary(&gid3, &bn3)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|m| {
                        let cat = m.auto_category.unwrap_or_else(|| "uncategorized".into());
                        (m.name, cat)
                    })
                    .collect();

                // Use cleaner's categorize_file for consistent file type detection

                // Group conflicts by mod pair (sorted pair of mod names)
                struct PairInfo {
                    mod_a: String,
                    mod_b: String,
                    type_counts: std::collections::HashMap<String, usize>,
                    winner: String,
                    same_collection: bool,
                }

                let mut pairs: std::collections::HashMap<(String, String), PairInfo> =
                    std::collections::HashMap::new();

                for c in &conflicts {
                    let file_cat = cleaner::categorize_file(&gid3, &c.relative_path);
                    let winner_name = c
                        .mods
                        .iter()
                        .find(|m| m.mod_id == c.winner_mod_id)
                        .map(|m| m.mod_name.clone())
                        .unwrap_or_default();

                    // For each pair of conflicting mods in this file
                    for i in 0..c.mods.len() {
                        for j in (i + 1)..c.mods.len() {
                            let (a, b) = if c.mods[i].mod_name <= c.mods[j].mod_name {
                                (c.mods[i].mod_name.clone(), c.mods[j].mod_name.clone())
                            } else {
                                (c.mods[j].mod_name.clone(), c.mods[i].mod_name.clone())
                            };
                            let key = (a.clone(), b.clone());
                            let entry = pairs.entry(key).or_insert_with(|| PairInfo {
                                mod_a: a,
                                mod_b: b,
                                type_counts: std::collections::HashMap::new(),
                                winner: winner_name.clone(),
                                same_collection: c.same_collection,
                            });
                            *entry.type_counts.entry(file_cat.clone()).or_insert(0) += 1;
                        }
                    }
                }

                // Sort pairs by total conflict count descending
                let mut pair_list: Vec<PairInfo> = pairs.into_values().collect();
                pair_list.sort_by(|a, b| {
                    let total_a: usize = a.type_counts.values().sum();
                    let total_b: usize = b.type_counts.values().sum();
                    total_b.cmp(&total_a)
                });

                let total_conflicts = conflicts.len();
                let total_pairs = pair_list.len();
                let mut output = format!(
                    "{} conflicts found between {} mod pair{}:\n",
                    total_conflicts,
                    total_pairs,
                    if total_pairs == 1 { "" } else { "s" }
                );

                for (i, pair) in pair_list.iter().enumerate() {
                    if i >= 15 {
                        output.push_str(&format!(
                            "\n...and {} more mod pairs",
                            total_pairs - 15
                        ));
                        break;
                    }
                    let total: usize = pair.type_counts.values().sum();
                    let cat_a = mod_categories
                        .get(&pair.mod_a)
                        .map(|s| s.as_str())
                        .unwrap_or("uncategorized");
                    let cat_b = mod_categories
                        .get(&pair.mod_b)
                        .map(|s| s.as_str())
                        .unwrap_or("uncategorized");

                    // Format type breakdown: "3 Mesh, 2 Texture"
                    let mut types: Vec<(&String, &usize)> = pair.type_counts.iter().collect();
                    types.sort_by(|a, b| b.1.cmp(a.1));
                    let type_str: Vec<String> = types
                        .iter()
                        .map(|(t, n)| format!("{} {}", n, t))
                        .collect();

                    output.push_str(&format!(
                        "\n{} vs {} ({} conflict{}):\n  - {} ({} vs {})\n  - Winner: {}{}\n",
                        pair.mod_a,
                        pair.mod_b,
                        total,
                        if total == 1 { "" } else { "s" },
                        type_str.join(", "),
                        cat_a,
                        cat_b,
                        pair.winner,
                        if pair.same_collection {
                            " [same collection - expected overlap]"
                        } else {
                            ""
                        },
                    ));
                }

                output.push_str(
                    "\nHigher-priority mod wins file conflicts. Use sort_load_order for plugin ordering.",
                );
                output
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "web_search" => {
            let query = args
                .get("query")
                .and_then(|q| q.as_str())
                .unwrap_or("")
                .to_string();
            let r = web_search_ddg(&query).await;
            (r, true)
        }

        "search_nexus" => {
            let query = args
                .get("query")
                .and_then(|q| q.as_str())
                .unwrap_or("")
                .to_string();
            // Map LLM-friendly sort names to NexusMods GraphQL field names
            let sort = args.get("sort_by").and_then(|s| s.as_str()).map(|s| {
                match s {
                    "total_downloads" | "downloads" => "downloads",
                    "latest_updated" | "updated" => "updatedAt",
                    "endorsements" | "endorsement_count" => "endorsements",
                    other => other,
                }
                .to_string()
            });
            let include_adult = args
                .get("include_adult")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let game_slug = nexus_game_slug(&gid);
            let r = match search_nexus_mods_cmd(
                game_slug,
                Some(query),
                sort,
                None,
                10,
                0,
                include_adult,
                None,
                None,
                None,
                None,
                None,
            )
            .await
            {
                Ok(result) => {
                    // Build structured data for rich mod cards
                    let cards: Vec<serde_json::Value> = result
                        .mods
                        .iter()
                        .map(|m| {
                            serde_json::json!({
                                "mod_id": m.mod_id,
                                "name": m.name,
                                "summary": m.summary,
                                "author": m.author,
                                "picture_url": m.picture_url,
                                "unique_downloads": m.unique_downloads,
                                "endorsements": m.endorsement_count,
                            })
                        })
                        .collect();
                    if !cards.is_empty() {
                        structured_data = Some(serde_json::json!(cards));
                    }
                    let lines: Vec<String> = result
                        .mods
                        .iter()
                        .map(|m| {
                            format!(
                                "[{}] {} — {} downloads — {}",
                                m.mod_id,
                                m.name,
                                m.unique_downloads,
                                m.summary.chars().take(80).collect::<String>()
                            )
                        })
                        .collect();
                    if lines.is_empty() {
                        "No mods found matching that search.".into()
                    } else {
                        format!("{} results:\n{}", lines.len(), lines.join("\n"))
                    }
                }
                Err(e) => format!("Search error: {e}"),
            };
            (r, true)
        }

        "get_nexus_mod_detail" => {
            let mod_id = args.get("mod_id").and_then(|v| v.as_i64()).unwrap_or(0);
            let game_slug = nexus_game_slug(&gid);
            let r = match get_nexus_mod_detail(game_slug, mod_id).await {
                Ok(info) => format!(
                    "Name: {}\nAuthor: {}\nVersion: {}\nDownloads: {}\nEndorsements: {}\nSummary: {}\nDescription: {}",
                    info.name,
                    info.author,
                    info.version,
                    info.unique_downloads,
                    info.endorsement_count,
                    info.summary,
                    info.description.as_deref().unwrap_or("").chars().take(500).collect::<String>(),
                ),
                Err(e) => format!("Error: {e}"),
            };
            (r, true)
        }

        "get_nexus_mod_files" => {
            let mod_id = args.get("mod_id").and_then(|v| v.as_i64()).unwrap_or(0);
            let game_slug = nexus_game_slug(&gid);
            let r = match get_nexus_mod_files(game_slug, mod_id).await {
                Ok(files) => {
                    let lines: Vec<String> = files
                        .iter()
                        .map(|f| format!("[{}] {} ({} KB)", f.file_id, f.name, f.size_kb))
                        .collect();
                    format!("{} files:\n{}", lines.len(), lines.join("\n"))
                }
                Err(e) => format!("Error: {e}"),
            };
            (r, true)
        }

        "check_mod_updates" => {
            let db2 = state.db.clone();
            let gid2 = gid.clone();
            let bn2 = bn.clone();
            let r = match tokio::task::spawn_blocking(move || {
                let mods = db2
                    .list_mods_summary(&gid2, &bn2)
                    .map_err(|e| e.to_string())?;
                let nexus_mods: Vec<_> = mods
                    .iter()
                    .filter_map(|m| {
                        m.nexus_mod_id
                            .map(|nid| (m.name.clone(), m.version.clone(), nid))
                    })
                    .collect();
                Ok::<_, String>(nexus_mods)
            })
            .await
            {
                Ok(Ok(nexus_mods)) => {
                    if nexus_mods.is_empty() {
                        "No mods with Nexus IDs to check.".into()
                    } else {
                        format!("{} mods have Nexus IDs. Use get_nexus_mod_detail to check individual mod versions.", nexus_mods.len())
                    }
                }
                Ok(Err(e)) => format!("Error: {e}"),
                Err(e) => format!("Error: {e}"),
            };
            (r, true)
        }

        "get_mod_recommendations" => {
            let mod_name = args
                .get("mod_name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let r = tokio::task::spawn_blocking({
                let db2 = db.clone();
                let gid2 = gid.clone();
                let bn2 = bn.clone();
                move || {
                    let mods = db2.list_mods_summary(&gid2, &bn2).unwrap_or_default();
                    match find_mod_by_name(&mods, &mod_name) {
                        Some(m) => match get_mod_recommendations_sync(&db2, &gid2, &bn2, m.id) {
                            Ok(recs) => {
                                if recs.is_empty() {
                                    format!("No recommendations found for \"{}\"", m.name)
                                } else {
                                    let lines: Vec<String> = recs
                                        .iter()
                                        .take(10)
                                        .map(|r| format!("{} (score: {})", r.0, r.1))
                                        .collect();
                                    format!(
                                        "Recommended with \"{}\":\n{}",
                                        m.name,
                                        lines.join("\n")
                                    )
                                }
                            }
                            Err(e) => format!("Error: {e}"),
                        },
                        None => format!("Mod \"{}\" not found", mod_name),
                    }
                }
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "get_popular_companion_mods" => {
            let r =
                tokio::task::spawn_blocking(move || {
                    match mod_recommendations::get_popular_mods(&db, &gid, &bn) {
                        Ok(popular) => {
                            let lines: Vec<String> = popular
                                .iter()
                                .take(15)
                                .map(|(name, _nexus_id, count)| {
                                    format!("{} (installed by {} users)", name, count)
                                })
                                .collect();
                            if lines.is_empty() {
                                "No popularity data available.".into()
                            } else {
                                format!("Popular mods:\n{}", lines.join("\n"))
                            }
                        }
                        Err(e) => format!("Error: {e}"),
                    }
                })
                .await
                .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        // ── Advanced: install, sort, crash analysis, profiles ─────────
        "download_and_install_mod" => {
            let mod_id = args.get("mod_id").and_then(|v| v.as_i64()).unwrap_or(0);
            let file_id = args.get("file_id").and_then(|v| v.as_i64()).unwrap_or(0);
            (format!("To install mod {} (file {}), please use the Downloads tab in the UI. I can help you find the right mod and file ID using search_nexus and get_nexus_mod_files.", mod_id, file_id), true)
        }

        "sort_load_order" => {
            let r = match sort_plugins_loot(gid.clone(), bn.clone()).await {
                Ok(result) => {
                    format!(
                        "Load order sorted. {} plugins reordered, {} warnings.",
                        result.plugins_moved,
                        result.warnings.len()
                    )
                }
                Err(e) => format!("Sort failed: {e}"),
            };
            (r, true)
        }

        "get_crash_logs" => {
            let r = match find_crash_logs_cmd(gid.clone(), bn.clone()).await {
                Ok(logs) => {
                    if logs.is_empty() {
                        "No crash logs found.".into()
                    } else {
                        let lines: Vec<String> = logs
                            .iter()
                            .take(5)
                            .map(|l| format!("{}: {} — {}", l.timestamp, l.filename, l.summary))
                            .collect();
                        format!("{} crash logs found:\n{}", logs.len(), lines.join("\n"))
                    }
                }
                Err(e) => format!("Error: {e}"),
            };
            (r, true)
        }

        "analyze_crash_log" => {
            let log_path = args
                .get("log_path")
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string();
            let r = match analyze_crash_log_cmd(log_path).await {
                Ok(report) => {
                    let diagnoses: Vec<String> = report
                        .diagnosis
                        .iter()
                        .map(|d| format!("  - {}: {}", d.title, d.description))
                        .collect();
                    format!(
                        "Crash Analysis:\nModule: {}\nPlugins involved: {}\nDiagnosis:\n{}",
                        report.module_name,
                        if report.involved_plugins.is_empty() {
                            "none".into()
                        } else {
                            report.involved_plugins.join(", ")
                        },
                        if diagnoses.is_empty() {
                            "  No specific diagnosis.".into()
                        } else {
                            diagnoses.join("\n")
                        },
                    )
                }
                Err(e) => format!("Analysis failed: {e}"),
            };
            (r, true)
        }

        "list_profiles" => {
            let db2 = state.db.clone();
            let gid2 = gid.clone();
            let bn2 = bn.clone();
            let r = tokio::task::spawn_blocking(move || {
                match profiles::list_profiles(&db2, &gid2, &bn2) {
                    Ok(profs) => {
                        let lines: Vec<String> = profs
                            .iter()
                            .map(|p| {
                                format!("{}{}", p.name, if p.is_active { " (active)" } else { "" })
                            })
                            .collect();
                        if lines.is_empty() {
                            "No profiles created.".into()
                        } else {
                            format!("{} profiles:\n{}", lines.len(), lines.join("\n"))
                        }
                    }
                    Err(e) => format!("Error: {e}"),
                }
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "activate_profile" => {
            let profile_name = args
                .get("profile_name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let db2 = state.db.clone();
            let gid2 = gid.clone();
            let bn2 = bn.clone();
            let r = tokio::task::spawn_blocking(move || {
                let profs =
                    profiles::list_profiles(&db2, &gid2, &bn2).map_err(|e| e.to_string())?;
                let lower = profile_name.to_lowercase();
                match profs.iter().find(|p| p.name.to_lowercase() == lower) {
                    Some(p) => {
                        profiles::set_active_profile(&db2, &gid2, &bn2, p.id)
                            .map_err(|e| e.to_string())?;
                        Ok(format!("Activated profile \"{}\"", p.name))
                    }
                    None => Ok(format!("Profile \"{}\" not found", profile_name)),
                }
            })
            .await
            .unwrap_or_else(|e| Ok(format!("Error: {e}")))
            .unwrap_or_else(|e: String| format!("Error: {e}"));
            (r, true)
        }

        "run_preflight_check" => {
            let db2 = state.db.clone();
            let gid2 = gid.clone();
            let bn2 = bn.clone();
            let r = tokio::task::spawn_blocking(move || {
                let (bottle, _, data_dir) = resolve_game(&gid2, &bn2)?;
                let result = preflight::run_preflight(&db2, &bottle, &gid2, &bn2, &data_dir);
                let mut lines = Vec::new();
                lines.push(format!(
                    "{} passed, {} failed, {} warnings",
                    result.passed, result.failed, result.warnings
                ));
                if result.can_proceed {
                    lines.push("Can proceed with launch.".into());
                } else {
                    lines.push("Issues must be resolved before launch.".into());
                }
                for check in &result.checks {
                    lines.push(format!(
                        "  [{:?}] {}: {}",
                        check.status, check.name, check.message
                    ));
                }
                Ok::<String, String>(lines.join("\n"))
            })
            .await
            .unwrap_or_else(|e| Ok(format!("Error: {e}")))
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "check_dependency_issues" => {
            let db2 = state.db.clone();
            let gid2 = gid.clone();
            let bn2 = bn.clone();
            let r =
                tokio::task::spawn_blocking(
                    move || match mod_dependencies::check_dependency_issues(&db2, &gid2, &bn2) {
                        Ok(issues) => {
                            if issues.is_empty() {
                                "No dependency issues found.".into()
                            } else {
                                let lines: Vec<String> = issues
                                    .iter()
                                    .take(15)
                                    .map(|i| format!("{}: {}", i.mod_name, i.message))
                                    .collect();
                                format!("{} issues:\n{}", issues.len(), lines.join("\n"))
                            }
                        }
                        Err(e) => format!("Error: {e}"),
                    },
                )
                .await
                .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "redeploy_mods" => {
            let db2 = state.db.clone();
            let gid2 = gid.clone();
            let bn2 = bn.clone();
            let r = tokio::task::spawn_blocking(move || {
                let (_bottle, game, data_dir) = resolve_game(&gid2, &bn2)?;
                let game_path = game.game_path.clone();
                match deployer::redeploy_all(&db2, &gid2, &bn2, &data_dir, &game_path) {
                    Ok(result) => Ok(format!(
                        "Redeployment complete: {} files deployed, {} skipped",
                        result.deployed_count, result.skipped_count
                    )),
                    Err(e) => Err(format!("Redeploy failed: {e}")),
                }
            })
            .await
            .unwrap_or_else(|e| Ok(format!("Error: {e}")))
            .unwrap_or_else(|e| e);
            (r, true)
        }

        "get_mod_requirements" => {
            let mod_id = args.get("mod_id").and_then(|v| v.as_i64()).unwrap_or(0);
            let game_slug = nexus_game_slug(&gid);

            let mod_info = get_nexus_mod_detail(game_slug, mod_id).await;
            let r = match mod_info {
                Ok(info) => {
                    let full_text = format!(
                        "{} {}",
                        info.summary,
                        info.description.as_deref().unwrap_or("")
                    );
                    let text_lower = full_text.to_lowercase();

                    // Strip HTML tags for cleaner matching
                    let tag_re = regex::Regex::new(r"<[^>]+>")
                        .unwrap_or_else(|_| regex::Regex::new("$^").unwrap());
                    let text_clean = tag_re.replace_all(&text_lower, " ").to_string();

                    let has_req_context = text_clean.contains("require")
                        || text_clean.contains("requirement")
                        || text_clean.contains("dependenc")
                        || text_clean.contains("you need")
                        || text_clean.contains("prerequisite")
                        || text_clean.contains("needed")
                        || text_clean.contains("must have")
                        || text_clean.contains("install first");

                    // Get installed mod names for cross-referencing
                    let db2 = state.db.clone();
                    let gid2 = gid.clone();
                    let bn2 = bn.clone();
                    let installed_mods = tokio::task::spawn_blocking(move || {
                        db2.list_mods_summary(&gid2, &bn2).unwrap_or_default()
                    })
                    .await
                    .unwrap_or_default();

                    let installed_names_lower: Vec<String> = installed_mods
                        .iter()
                        .map(|m| m.name.to_lowercase())
                        .collect();

                    let mut detected: Vec<(&str, i64, bool)> = Vec::new();

                    for &(fw_name, fw_nexus_id) in KNOWN_FRAMEWORKS {
                        let fw_lower = fw_name.to_lowercase();

                        let mentioned = text_clean.contains(&fw_lower) || {
                            let short = fw_lower
                                .split(|c: char| c == '-' || c == ' ')
                                .next()
                                .unwrap_or(&fw_lower);
                            short.len() > 3 && text_clean.contains(short)
                        };

                        if mentioned {
                            let is_installed = installed_names_lower.iter().any(|installed| {
                                installed.contains(&fw_lower)
                                    || fw_lower.contains(installed.as_str())
                                    || {
                                        let short = fw_lower
                                            .split(|c: char| c == '-' || c == ' ')
                                            .next()
                                            .unwrap_or(&fw_lower);
                                        short.len() > 3 && installed.contains(short)
                                    }
                            });

                            if !detected.iter().any(|(_, nid, _)| *nid == fw_nexus_id) {
                                detected.push((fw_name, fw_nexus_id, is_installed));
                            }
                        }
                    }

                    let mut out =
                        format!("Requirements for {} (Nexus ID {}):\n", info.name, mod_id);

                    if detected.is_empty() {
                        if has_req_context {
                            out.push_str(
                                "The mod mentions requirements but none matched known frameworks.\nCheck the mod page manually for specific requirements.\n",
                            );
                        } else {
                            out.push_str("No known framework dependencies detected.\n");
                        }
                    } else {
                        let mut missing_count = 0;
                        for (fw_name, fw_nexus_id, is_installed) in &detected {
                            if *is_installed {
                                out.push_str(&format!("[installed] {}\n", fw_name));
                            } else {
                                missing_count += 1;
                                out.push_str(&format!(
                                    "[MISSING]   {} — NOT installed (Nexus ID: {})\n",
                                    fw_name, fw_nexus_id
                                ));
                            }
                        }
                        if missing_count > 0 {
                            out.push_str(
                                "\nUse open_nexus_mod to show missing mods for installation.",
                            );
                        } else {
                            out.push_str("\nAll detected dependencies are installed.");
                        }
                    }

                    out
                }
                Err(e) => format!("Error fetching mod details: {e}"),
            };
            (r, true)
        }

        "find_needed_patches" => {
            let r = tokio::task::spawn_blocking(move || {
                let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
                let enabled: Vec<_> = mods.iter().filter(|m| m.enabled).collect();
                let total = enabled.len();

                // Group by auto_category
                let mut groups: std::collections::HashMap<String, Vec<String>> =
                    std::collections::HashMap::new();
                for m in &enabled {
                    let cat = m
                        .auto_category
                        .as_deref()
                        .unwrap_or("uncategorized")
                        .to_string();
                    groups.entry(cat).or_default().push(m.name.clone());
                }

                // Only keep categories with 2+ mods (potential conflicts)
                let mut conflict_groups: Vec<(String, Vec<String>)> = groups
                    .into_iter()
                    .filter(|(cat, members)| {
                        members.len() >= 2 && cat != "uncategorized"
                    })
                    .collect();
                conflict_groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

                let mut out = format!(
                    "Analyzing {} enabled mods for patch needs...\n\n",
                    total
                );

                if conflict_groups.is_empty() {
                    out.push_str(
                        "No multi-mod category groups found. Mods may still need patches — use your knowledge of Skyrim modding to identify common pairs that need compatibility patches, then search_nexus for them.",
                    );
                } else {
                    out.push_str("Potential conflict groups:\n");
                    let mut suggestions = Vec::new();
                    for (cat, members) in &conflict_groups {
                        out.push_str(&format!(
                            "- {} ({} mods): {}\n",
                            cat,
                            members.len(),
                            members.join(", ")
                        ));
                        // Generate search suggestions for pairs within the group
                        if members.len() >= 2 {
                            for i in 0..members.len().min(3) {
                                for j in (i + 1)..members.len().min(4) {
                                    suggestions.push(format!(
                                        "\"{}\" \"{}\" patch",
                                        members[i], members[j]
                                    ));
                                }
                            }
                        }
                    }
                    if !suggestions.is_empty() {
                        out.push_str("\nSuggested searches:\n");
                        for s in suggestions.iter().take(8) {
                            out.push_str(&format!("- {}\n", s));
                        }
                    }
                    out.push_str("\nUse search_nexus to find patches for these combinations, then open_nexus_mod to show them to the user.");
                }

                out
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "batch_mod_operation" => {
            let action = args.get("action").and_then(|a| a.as_str()).unwrap_or("disable").to_string();
            let filter_type = args.get("filter_type").and_then(|f| f.as_str()).unwrap_or("").to_string();
            let filter_value = args.get("filter_value").and_then(|f| f.as_str()).unwrap_or("").to_string();
            let enable = action == "enable";
            let r = tokio::task::spawn_blocking(move || {
                let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
                let filter_lower = filter_value.to_lowercase();

                // Apply filter
                let filtered: Vec<&database::ModSummary> = mods.iter().filter(|m| {
                    let type_match = match filter_type.as_str() {
                        "category" => m.auto_category.as_deref().unwrap_or("").to_lowercase().contains(&filter_lower),
                        "collection" => m.collection_name.as_deref().unwrap_or("").to_lowercase() == filter_lower,
                        "name_contains" => m.name.to_lowercase().contains(&filter_lower),
                        "optional" => m.collection_optional,
                        "all_disabled" => !m.enabled,
                        "all_enabled" => m.enabled,
                        _ => false,
                    };
                    // Only include mods that would actually change state
                    type_match && (m.enabled != enable)
                }).collect();

                if filtered.is_empty() {
                    return "No mods match that filter (or they are already in the desired state).".to_string();
                }

                // Execute batch operation
                let mut changed = Vec::new();
                let mut errors = Vec::new();
                for m in &filtered {
                    match db.set_enabled(m.id, enable) {
                        Ok(_) => changed.push(m.name.clone()),
                        Err(e) => errors.push(format!("{}: {}", m.name, e)),
                    }
                }

                let action_past = if enable { "Enabled" } else { "Disabled" };
                let filter_desc = match filter_type.as_str() {
                    "category" => format!("{} mods", filter_value),
                    "collection" => format!("mods from \"{}\"", filter_value),
                    "name_contains" => format!("mods matching \"{}\"", filter_value),
                    "optional" => "optional mods".to_string(),
                    "all_disabled" => "all previously disabled mods".to_string(),
                    "all_enabled" => "all previously enabled mods".to_string(),
                    _ => "mods".to_string(),
                };

                let mut out = format!("{} {} {}:\n", action_past, changed.len(), filter_desc);
                let show_count = changed.len().min(10);
                for name in &changed[..show_count] {
                    out.push_str(&format!("- {}\n", name));
                }
                if changed.len() > show_count {
                    out.push_str(&format!("- ... (and {} more)\n", changed.len() - show_count));
                }
                if !errors.is_empty() {
                    out.push_str(&format!("\n{} errors:\n", errors.len()));
                    for e in &errors {
                        out.push_str(&format!("- {}\n", e));
                    }
                }
                let reverse_action = if enable { "disable" } else { "enable" };
                out.push_str(&format!("\nTo undo, ask me to \"{}\" these mods.", reverse_action));
                // Trigger redeploy notification
                out.push_str("\n\nNote: Run redeploy_mods to apply changes to the game directory.");
                out
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }


        "run_full_diagnostic" => {
            // Run all diagnostic checks in parallel
            let db_pf = state.db.clone();
            let gid_pf = gid.clone();
            let bn_pf = bn.clone();
            let preflight_fut = tokio::task::spawn_blocking(move || {
                let (bottle, _, data_dir) = resolve_game(&gid_pf, &bn_pf)?;
                let result =
                    preflight::run_preflight(&db_pf, &bottle, &gid_pf, &bn_pf, &data_dir);
                let status = if result.failed > 0 {
                    "FAIL"
                } else if result.warnings > 0 {
                    "WARN"
                } else {
                    "PASS"
                };
                let mut lines = vec![
                    format!("PREFLIGHT: [{}]", status),
                    format!(
                        "- {} passed, {} failed, {} warnings",
                        result.passed, result.failed, result.warnings
                    ),
                ];
                for check in &result.checks {
                    lines.push(format!(
                        "- [{:?}] {}: {}",
                        check.status, check.name, check.message
                    ));
                }
                Ok::<String, String>(lines.join("\n"))
            });

            // Wine compat + mod summary share a single list_mods_summary call
            let db_wc = state.db.clone();
            let gid_wc = gid.clone();
            let bn_wc = bn.clone();
            let wine_and_summary_fut = tokio::task::spawn_blocking(move || {
                let mods = db_wc
                    .list_mods_summary(&gid_wc, &bn_wc)
                    .unwrap_or_default();
                let enabled = mods.iter().filter(|m| m.enabled).count();
                let mod_summary = format!("MOD SUMMARY: {} enabled / {} total", enabled, mods.len());

                let compat_input = wine_compat::build_compat_input(&mods);
                let warnings = wine_compat::check_all_mods_wine_compat(&compat_input);
                let wine_result = if warnings.is_empty() {
                    "WINE COMPATIBILITY: [OK]\n- No issues detected".to_string()
                } else {
                    format!(
                        "WINE COMPATIBILITY: [WARNINGS]\n{}",
                        wine_compat::format_warnings_report(&warnings)
                    )
                };
                (wine_result, mod_summary)
            });

            let db_dep = state.db.clone();
            let gid_dep = gid.clone();
            let bn_dep = bn.clone();
            let dep_fut = tokio::task::spawn_blocking(move || {
                match mod_dependencies::check_dependency_issues(&db_dep, &gid_dep, &bn_dep) {
                    Ok(issues) => {
                        if issues.is_empty() {
                            "DEPENDENCIES: [OK]\n- No issues found".into()
                        } else {
                            let lines: Vec<String> = issues
                                .iter()
                                .take(15)
                                .map(|i| format!("- {}: {}", i.mod_name, i.message))
                                .collect();
                            format!(
                                "DEPENDENCIES: [ISSUES] ({} found)\n{}",
                                issues.len(),
                                lines.join("\n")
                            )
                        }
                    }
                    Err(e) => format!("DEPENDENCIES: [ERROR]\n- {e}"),
                }
            });

            let db_cf = state.db.clone();
            let gid_cf = gid.clone();
            let bn_cf = bn.clone();
            let conflict_fut = tokio::task::spawn_blocking(move || {
                match db_cf
                    .find_all_conflicts(&gid_cf, &bn_cf)
                    .map_err(|e| e.to_string())
                {
                    Ok(conflicts) => {
                        if conflicts.is_empty() {
                            "CONFLICTS: 0 total".into()
                        } else {
                            // Group by mod pair for consistency with get_conflicts output
                            let mut pair_counts: std::collections::HashMap<(String, String), usize> =
                                std::collections::HashMap::new();
                            for c in &conflicts {
                                for i in 0..c.mods.len() {
                                    for j in (i + 1)..c.mods.len() {
                                        let (a, b) = if c.mods[i].mod_name <= c.mods[j].mod_name {
                                            (c.mods[i].mod_name.clone(), c.mods[j].mod_name.clone())
                                        } else {
                                            (c.mods[j].mod_name.clone(), c.mods[i].mod_name.clone())
                                        };
                                        *pair_counts.entry((a, b)).or_insert(0) += 1;
                                    }
                                }
                            }
                            let mut pairs: Vec<_> = pair_counts.into_iter().collect();
                            pairs.sort_by(|a, b| b.1.cmp(&a.1));
                            let lines: Vec<String> = pairs
                                .iter()
                                .take(10)
                                .map(|((a, b), count)| format!("- {} vs {} ({} files)", a, b, count))
                                .collect();
                            format!(
                                "CONFLICTS: {} total, {} mod pairs\n{}",
                                conflicts.len(),
                                pairs.len(),
                                lines.join("\n")
                            )
                        }
                    }
                    Err(e) => format!("CONFLICTS: [ERROR]\n- {e}"),
                }
            });

            // Await all in parallel
            let (preflight_r, wine_summary_r, dep_r, conflict_r) =
                tokio::join!(preflight_fut, wine_and_summary_fut, dep_fut, conflict_fut);

            let preflight_result = preflight_r
                .unwrap_or_else(|e| Ok(format!("PREFLIGHT: [ERROR]\n- {e}")))
                .unwrap_or_else(|e| format!("PREFLIGHT: [ERROR]\n- {e}"));
            let (wine_result, mod_summary) = wine_summary_r
                .unwrap_or_else(|_| ("WINE COMPATIBILITY: [ERROR]".into(), "MOD SUMMARY: [ERROR]".into()));
            let dep_result = dep_r
                .unwrap_or_else(|e| format!("DEPENDENCIES: [ERROR]\n- {e}"));
            let conflict_result = conflict_r
                .unwrap_or_else(|e| format!("CONFLICTS: [ERROR]\n- {e}"));

            // Determine highest severity area for focus recommendation
            let focus = if preflight_result.contains("[FAIL]") {
                "preflight failures (must be resolved before launching)"
            } else if wine_result.contains("[WARNINGS]") {
                "Wine compatibility warnings (may cause crashes)"
            } else if dep_result.contains("[ISSUES]") {
                "dependency issues (missing or circular dependencies)"
            } else if !conflict_result.starts_with("CONFLICTS: 0") {
                "file conflicts (may cause unexpected behavior)"
            } else {
                "no major issues detected \u{2014} check crash logs if problem persists"
            };

            let r = format!(
                "=== FULL DIAGNOSTIC REPORT ===\n\n{}\n\n{}\n\n{}\n\n{}\n\n{}\n\nBased on this report, focus investigation on {}.",
                preflight_result, wine_result, dep_result, conflict_result, mod_summary, focus
            );
            (r, true)
        }

        "get_mod_health" => {
            let db2 = state.db.clone();
            let gid2 = gid.clone();
            let bn2 = bn.clone();
            let r = tokio::task::spawn_blocking(move || {
                let mut score: i32 = 100;
                let mut issues: Vec<serde_json::Value> = Vec::new();

                let mods = db2.list_mods_summary(&gid2, &bn2).unwrap_or_default();
                let enabled_count = mods.iter().filter(|m| m.enabled).count();

                // 1. Check Wine compatibility
                let compat_input = wine_compat::build_compat_input(&mods);
                let warnings = wine_compat::check_all_mods_wine_compat(&compat_input);
                for (mod_name, warning) in &warnings {
                    let (penalty, severity_str) = match warning.severity {
                        wine_compat::Severity::Crash => (30, "error"),
                        wine_compat::Severity::Broken => (15, "warning"),
                        wine_compat::Severity::Degraded => (5, "info"),
                    };
                    score -= penalty;
                    issues.push(serde_json::json!({
                        "severity": severity_str,
                        "message": format!("{} \u{2014} {}", mod_name, warning.reason),
                        "points": -penalty,
                    }));
                }

                // 2. Check dependencies (missing masters)
                match mod_dependencies::check_dependency_issues(&db2, &gid2, &bn2) {
                    Ok(dep_issues) => {
                        let missing: Vec<_> = dep_issues
                            .iter()
                            .filter(|i| {
                                i.issue_type
                                    == mod_dependencies::DependencyIssueType::MissingRequirement
                            })
                            .collect();
                        for issue in &missing {
                            score -= 20;
                            issues.push(serde_json::json!({
                                "severity": "error",
                                "message": format!("{}: {}", issue.mod_name, issue.message),
                                "points": -20,
                            }));
                        }
                    }
                    Err(_) => {} // Skip if dependency check fails
                }

                // 3. Check conflicts (capped at -30)
                match db2.find_all_conflicts(&gid2, &bn2) {
                    Ok(conflicts) => {
                        if !conflicts.is_empty() {
                            let penalty = (conflicts.len() as i32 * 2).min(30);
                            score -= penalty;
                            issues.push(serde_json::json!({
                                "severity": "info",
                                "message": format!("{} file conflicts between mods", conflicts.len()),
                                "points": -penalty,
                            }));
                        }
                    }
                    Err(_) => {}
                }

                // 4. Check mod count sanity
                if enabled_count == 0 {
                    score -= 50;
                    issues.push(serde_json::json!({
                        "severity": "warning",
                        "message": "No mods are enabled",
                        "points": -50,
                    }));
                }

                // Clamp score
                let score = score.clamp(0, 100);
                let color = if score >= 80 {
                    "green"
                } else if score >= 50 {
                    "yellow"
                } else {
                    "red"
                };

                let color_emoji = match color {
                    "green" => "\u{1F7E2}",
                    "yellow" => "\u{1F7E1}",
                    _ => "\u{1F534}",
                };

                // Build text summary
                let mut text = format!("Mod Health Score: {}/100 {}\n", score, color_emoji);
                if !issues.is_empty() {
                    text.push_str("\nIssues:\n");
                    for issue in &issues {
                        let icon = match issue["severity"].as_str().unwrap_or("info") {
                            "error" => "\u{274C}",
                            "warning" => "\u{26A0}\u{FE0F}",
                            _ => "\u{2139}\u{FE0F}",
                        };
                        text.push_str(&format!(
                            "{} {} ({})\n",
                            icon,
                            issue["message"].as_str().unwrap_or(""),
                            issue["points"]
                        ));
                    }
                }
                let overall = match color {
                    "green" => "Your mod setup looks healthy!",
                    "yellow" => "Your mod setup has some issues that could be improved.",
                    _ => "Your mod setup has significant issues that should be addressed.",
                };
                text.push_str(&format!("\nOverall: {}", overall));

                (score, color.to_string(), issues, text)
            })
            .await
            .unwrap_or_else(|e| (0, "red".to_string(), vec![], format!("Error: {e}")));

            let (health_score, health_color, health_issues, text) = r;
            structured_data = Some(serde_json::json!({
                "type": "health_score",
                "score": health_score,
                "color": health_color,
                "issues": health_issues,
            }));
            (text, true)
        }

        _ => (format!("Unknown tool: {name}"), false),
    };

    let display_name = tool_result_display_name(name, &result);
    llm_chat::ToolResult {
        tool_name: name.into(),
        result,
        success,
        display_name,
        structured_data,
    }
}

/// Map game_id to NexusMods game slug.
pub fn nexus_game_slug(game_id: &str) -> String {
    match game_id {
        "skyrimse" | "skyrimspecialedition" => "skyrimspecialedition".into(),
        "skyrim" => "skyrim".into(),
        "fallout4" => "fallout4".into(),
        "starfield" => "starfield".into(),
        "oblivion" => "oblivion".into(),
        "morrowind" => "morrowind".into(),
        other => other.to_string(),
    }
}

/// Sync wrapper for mod recommendations (used from spawn_blocking).
pub fn get_mod_recommendations_sync(
    db: &std::sync::Arc<database::ModDatabase>,
    game_id: &str,
    bottle_name: &str,
    _mod_id: i64,
) -> Result<Vec<(String, i64, usize)>, String> {
    mod_recommendations::get_popular_mods(db, game_id, bottle_name)
}

