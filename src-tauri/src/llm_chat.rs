//! Local LLM chat system with tool calling and auto lifecycle management.
//!
//! Supports two backends:
//! - **Ollama** (llama.cpp + Metal) — cross-platform, uses `/api/chat`
//! - **MLX LM** (Apple MLX) — macOS Apple Silicon only, ~2x faster, uses OpenAI-compatible `/v1/chat/completions`
//!
//! The LLM can call tools to interact with the mod manager.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::instruction_types::{ModelCapabilityTier, OllamaModel};

// ---------------------------------------------------------------------------
// Backend selection
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LlmBackend {
    Ollama,
    Mlx,
}

impl Default for LlmBackend {
    fn default() -> Self {
        Self::Ollama
    }
}

impl LlmBackend {
    pub fn base_url(&self) -> &str {
        match self {
            Self::Ollama => "http://localhost:11434",
            Self::Mlx => "http://localhost:8080",
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            Self::Ollama => "Ollama",
            Self::Mlx => "MLX",
        }
    }
}

/// Map a recommended model name to the MLX-community HuggingFace model ID.
/// MLX uses Qwen3.5 (tool calling works via OpenAI-compat API with correct format).
/// Ollama uses Qwen3 (Hermes format that Ollama's pipeline supports).
pub fn mlx_model_name(model_name: &str) -> String {
    match model_name {
        // Map Ollama Qwen3 names → MLX Qwen3.5 equivalents (better tool calling on MLX)
        "qwen3:32b" => "mlx-community/Qwen3.5-27B-4bit".into(), // OP Mode: dense 27B
        "qwen3:30b-a3b" => "mlx-community/Qwen3.5-35B-A3B-4bit".into(), // MoE 35B
        "qwen3:8b" => "mlx-community/Qwen3.5-9B-4bit".into(),
        "qwen3:4b" => "mlx-community/Qwen3.5-4B-4bit".into(),
        "qwen3:1.7b" => "mlx-community/Qwen3.5-2B-4bit".into(),
        other => other.to_string(), // Pass through for custom HuggingFace IDs
    }
}

/// Map an MLX model name back to a short display name.
pub fn ollama_model_name(mlx_name: &str) -> String {
    if mlx_name.contains("Qwen3.5-35B-A3B") {
        return "Qwen3.5 35B-A3B".into();
    }
    if mlx_name.contains("Qwen3.5-27B") {
        return "Qwen3.5 27B".into();
    }
    if mlx_name.contains("Qwen3.5-9B") {
        return "Qwen3.5 9B".into();
    }
    if mlx_name.contains("Qwen3.5-4B") {
        return "Qwen3.5 4B".into();
    }
    if mlx_name.contains("Qwen3.5-2B") {
        return "Qwen3.5 2B".into();
    }
    if mlx_name.contains("Qwen3-30B-A3B") {
        return "Qwen3 30B-A3B".into();
    }
    mlx_name.to_string()
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallResponse>>,
    /// Mods referenced in this message (populated by backend after response)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mentioned_mods: Option<Vec<MentionedMod>>,
}

/// A mod referenced in a chat message, with contextual action info.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MentionedMod {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nexus_mod_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    pub installed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallResponse {
    pub function: ToolCallFunction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: ChatToolFunction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatState {
    pub model: Option<String>,
    pub backend: LlmBackend,
    pub loaded: bool,
    pub messages: Vec<ChatMessage>,
    pub available_models: Vec<OllamaModel>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatResponse {
    pub message: ChatMessage,
    /// If the LLM called tools, these are the results ready to display.
    #[serde(default)]
    pub tool_results: Vec<ToolResult>,
    /// If true, destructive tools need user confirmation before executing.
    #[serde(default)]
    pub needs_confirmation: bool,
    /// Tool calls awaiting confirmation (only set when needs_confirmation is true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_tool_calls: Option<Vec<ToolCallResponse>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_name: String,
    pub result: String,
    pub success: bool,
    /// Human-friendly description of what this tool did.
    #[serde(default)]
    pub display_name: String,
    /// Structured data for rich rendering (e.g., Nexus mod cards).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_data: Option<serde_json::Value>,
}

/// Contextual conversation starter.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatStarter {
    pub label: String,
    pub prompt: String,
}

// ---------------------------------------------------------------------------
// Session manager
// ---------------------------------------------------------------------------

pub struct ChatSession {
    pub model: Option<String>,
    pub backend: LlmBackend,
    pub messages: Vec<ChatMessage>,
    pub last_activity: Instant,
    pub unload_timeout: Duration,
}

impl ChatSession {
    pub fn new() -> Self {
        Self {
            model: None,
            backend: LlmBackend::default(),
            messages: Vec::new(),
            last_activity: Instant::now(),
            unload_timeout: Duration::from_secs(300), // 5 minutes
        }
    }

    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    pub fn is_expired(&self) -> bool {
        self.last_activity.elapsed() > self.unload_timeout
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }
}

pub type SharedChatSession = Arc<Mutex<ChatSession>>;

pub fn create_shared_session() -> SharedChatSession {
    Arc::new(Mutex::new(ChatSession::new()))
}

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

/// Helper to define a tool concisely.
fn tool(name: &str, desc: &str, params: serde_json::Value) -> ChatTool {
    ChatTool {
        tool_type: "function".into(),
        function: ChatToolFunction {
            name: name.into(),
            description: desc.into(),
            parameters: params,
        },
    }
}

pub fn get_chat_tools(tier: ModelCapabilityTier) -> Vec<ChatTool> {
    let no_params = serde_json::json!({ "type": "object", "properties": {} });

    // ── Basic tier — all models ──────────────────────────────────────
    let mut tools = vec![
        tool("navigate_ui", "Navigate Corkscrew's UI to a page. Pages: discover, mods, plugins, profiles, logs, settings, dashboard.", serde_json::json!({
            "type": "object",
            "properties": { "page": { "type": "string", "description": "Page name: discover, mods, plugins, profiles, logs, settings, dashboard" } },
            "required": ["page"]
        })),
        tool("open_nexus_mod", "Open a NexusMods mod in Corkscrew's Discover tab with full detail view, images, and install button.", serde_json::json!({
            "type": "object",
            "properties": {
                "mod_id": { "type": "integer", "description": "NexusMods mod ID" },
                "name": { "type": "string", "description": "Mod name (for display)" }
            },
            "required": ["mod_id"]
        })),
        tool("list_mods", "List installed mods with status. Use filter to search.", serde_json::json!({
            "type": "object",
            "properties": {
                "filter": { "type": "string", "description": "Search filter" }
            }
        })),
        tool("enable_mod", "Enable a mod.", serde_json::json!({
            "type": "object",
            "properties": { "mod_name": { "type": "string" } },
            "required": ["mod_name"]
        })),
        tool("disable_mod", "Disable a mod.", serde_json::json!({
            "type": "object",
            "properties": { "mod_name": { "type": "string" } },
            "required": ["mod_name"]
        })),
        tool("get_mod_info", "Get mod details (version, category, files).", serde_json::json!({
            "type": "object",
            "properties": { "mod_name": { "type": "string" } },
            "required": ["mod_name"]
        })),
        tool("get_deployment_status", "Get deployment overview.", no_params.clone()),
        tool("web_search", "Search the web. Use for research, compatibility info, vague requests.", serde_json::json!({
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query"]
        })),
    ];

    // ── Standard tier — 3-4B+ models ─────────────────────────────────
    if tier >= ModelCapabilityTier::Standard {
        tools.extend([
            tool("get_load_order", "Get plugin load order.", no_params.clone()),
            tool("get_conflicts", "Get file conflicts between mods.", no_params.clone()),
            tool("search_nexus", "Search NexusMods for mods.", serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search text" },
                    "sort_by": { "type": "string", "description": "total_downloads|latest_updated|endorsements" },
                    "include_adult": { "type": "boolean" }
                },
                "required": ["query"]
            })),
            tool("get_nexus_mod_detail", "Get NexusMods mod details by ID.", serde_json::json!({
                "type": "object",
                "properties": { "mod_id": { "type": "integer" } },
                "required": ["mod_id"]
            })),
            tool("get_nexus_mod_files", "List files for a NexusMods mod.", serde_json::json!({
                "type": "object",
                "properties": { "mod_id": { "type": "integer" } },
                "required": ["mod_id"]
            })),
            tool("check_mod_updates", "Check for mod updates on NexusMods.", no_params.clone()),
            tool("get_mod_recommendations", "Get commonly co-installed mods.", serde_json::json!({
                "type": "object",
                "properties": { "mod_name": { "type": "string" } },
                "required": ["mod_name"]
            })),
            tool("get_popular_companion_mods", "Get popular mod combinations.", no_params.clone()),
        ]);
    }

    // ── Advanced tier — 7B+ models ───────────────────────────────────
    if tier >= ModelCapabilityTier::Advanced {
        tools.extend([
            tool(
                "download_and_install_mod",
                "Download and install from NexusMods.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "mod_id": { "type": "integer" },
                        "file_id": { "type": "integer" }
                    },
                    "required": ["mod_id", "file_id"]
                }),
            ),
            tool(
                "sort_load_order",
                "Auto-sort load order with LOOT.",
                no_params.clone(),
            ),
            tool(
                "get_crash_logs",
                "List recent crash logs.",
                no_params.clone(),
            ),
            tool(
                "analyze_crash_log",
                "Analyze a crash log.",
                serde_json::json!({
                    "type": "object",
                    "properties": { "log_path": { "type": "string" } },
                    "required": ["log_path"]
                }),
            ),
            tool("list_profiles", "List mod profiles.", no_params.clone()),
            tool(
                "activate_profile",
                "Switch mod profile.",
                serde_json::json!({
                    "type": "object",
                    "properties": { "profile_name": { "type": "string" } },
                    "required": ["profile_name"]
                }),
            ),
            tool(
                "run_preflight_check",
                "Pre-launch check (missing masters, Wine issues, SKSE).",
                no_params.clone(),
            ),
            tool(
                "check_dependency_issues",
                "Check for missing dependencies.",
                no_params.clone(),
            ),
            tool(
                "redeploy_mods",
                "Redeploy mod files to game directory.",
                no_params.clone(),
            ),
        ]);
    }

    tools
}

// ---------------------------------------------------------------------------
// System prompt for chat mode
// ---------------------------------------------------------------------------

pub fn build_chat_system_prompt(
    game_name: &str,
    mod_count: usize,
    platform: &str,
    current_page: &str,
    _readme_content: Option<&str>,
) -> String {
    // Page-specific focus hint (keeps the model on-task, saves tokens vs generic advice)
    let page_hint = match current_page {
        "Mods" => "User is managing mods. Focus on enable/disable, info, deployment.",
        "Load Order" => "User is on load order. Focus on plugin sorting, masters, patches.",
        "Discover" => "User is browsing. Help find, compare, and install mods from NexusMods.",
        "Crash Logs" => "User is debugging. Focus on crash analysis and diagnosis.",
        "Profiles" => "User is managing profiles. Help switch, compare, or create profiles.",
        "Settings" => "User is in settings. Answer config questions.",
        _ => "Help with whatever the user needs.",
    };

    format!(
        r#"You are a {game_name} modding expert in Corkscrew (mod manager for {platform}). Use tools to look things up — never guess mod names or IDs.

{mod_count} mods installed | Page: {current_page} | {page_hint}

Rules: Use tools proactively. Never guess — look it up. If search_nexus returns no results, ALWAYS fall back to web_search to find the mod name, then retry search_nexus. Max 5 tool calls. Be concise. You CONTROL Corkscrew's UI — use navigate_ui and open_nexus_mod to show things to the user directly.
SAFETY: For ANY destructive or hard-to-reverse action (uninstall mod, delete files, disable plugins, change load order, reset settings), ALWAYS ask the user to confirm before proceeding. Never auto-execute destructive actions.

Routing: find mod → search_nexus → open_nexus_mod (to show it in Corkscrew UI with install button) | install → search_nexus → open_nexus_mod | crash → get_crash_logs → analyze | conflicts → get_conflicts | vague → web_search → search_nexus → open_nexus_mod
When you find a mod the user wants, ALWAYS call open_nexus_mod to open it in Corkscrew's Discover tab where they can see images and install it.

Modding: .esm first, .esl light, .esp last. Patches after patched plugin. Later plugin wins record conflicts. Higher-priority mod wins file conflicts. Navmesh conflicts = game-breaking. LOOT for auto-sort. Wine: .NET Script Framework and original SSE Engine Fixes crash — Corkscrew ships Wine-compatible fork."#,
        game_name = game_name,
        platform = platform,
        mod_count = mod_count,
        current_page = current_page,
        page_hint = page_hint,
    )
}

// ---------------------------------------------------------------------------
// Tool prompt injection for MLX (no native tool calling support)
// ---------------------------------------------------------------------------

/// Format tool definitions as text to inject into the system prompt for MLX.
fn format_tools_for_prompt(tools: &[ChatTool]) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let mut s = String::from("\n\n## Available tools\nYou can call tools by writing a <tool_call> block. Format:\n<tool_call>\n{\"name\": \"tool_name\", \"arguments\": {\"arg\": \"value\"}}\n</tool_call>\n\nYou may call multiple tools. Available tools:\n");
    for t in tools {
        s.push_str(&format!(
            "\n### {}\n{}\nParameters: {}\n",
            t.function.name,
            t.function.description,
            serde_json::to_string(&t.function.parameters).unwrap_or_default()
        ));
    }
    s
}

/// Parse <tool_call> blocks from model text output (Qwen/Hermes format).
fn parse_tool_calls_from_text(text: &str) -> (String, Option<Vec<ToolCallResponse>>) {
    let mut calls = Vec::new();
    let mut clean_text = text.to_string();

    // Find all <tool_call>...</tool_call> blocks
    while let Some(start) = clean_text.find("<tool_call>") {
        let after_tag = start + "<tool_call>".len();
        if let Some(end) = clean_text[after_tag..].find("</tool_call>") {
            let json_str = clean_text[after_tag..after_tag + end].trim();
            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(json_str) {
                let name = obj
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let arguments = obj
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));
                if !name.is_empty() {
                    calls.push(ToolCallResponse {
                        function: ToolCallFunction { name, arguments },
                    });
                }
            }
            // Remove the tool_call block from text
            clean_text = format!(
                "{}{}",
                clean_text[..start].trim_end(),
                clean_text[after_tag + end + "</tool_call>".len()..].trim_start()
            );
        } else {
            break;
        }
    }

    let tool_calls = if calls.is_empty() { None } else { Some(calls) };
    (clean_text.trim().to_string(), tool_calls)
}

/// Inject tool definitions into the system message for MLX backend.
fn inject_tools_into_messages(messages: &[ChatMessage], tools: &[ChatTool]) -> Vec<ChatMessage> {
    if tools.is_empty() {
        return messages.to_vec();
    }
    let tool_text = format_tools_for_prompt(tools);
    let mut msgs = messages.to_vec();
    if let Some(system_msg) = msgs.iter_mut().find(|m| m.role == "system") {
        system_msg.content.push_str(&tool_text);
    }
    msgs
}

// ---------------------------------------------------------------------------
// Chat API — supports both Ollama and MLX LM backends
// ---------------------------------------------------------------------------

/// Send a message and get a response via the active backend.
pub async fn chat_send(
    backend: &LlmBackend,
    model: &str,
    messages: &[ChatMessage],
    tools: &[ChatTool],
    num_ctx: u32,
    max_tokens: u32,
) -> Result<ChatMessage, String> {
    match backend {
        LlmBackend::Ollama => chat_send_ollama(model, messages, tools, num_ctx, max_tokens).await,
        LlmBackend::Mlx => {
            // MLX LM server doesn't support native tool calling.
            // Inject tools into system prompt and parse tool_call tags from output.
            let msgs = inject_tools_into_messages(messages, tools);
            let mut response =
                chat_send_openai_compat(backend.base_url(), model, &msgs, &[], max_tokens).await?;
            let (clean_content, parsed_calls) = parse_tool_calls_from_text(&response.content);
            response.content = clean_content;
            if parsed_calls.is_some() {
                response.tool_calls = parsed_calls;
            }
            Ok(response)
        }
    }
}

/// Send a message with streaming — calls `on_token` for each chunk of text.
/// Returns the complete ChatMessage when done.
/// Phase of a streaming token — lets frontend distinguish thinking from answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamPhase {
    Thinking,
    Content,
}

pub async fn chat_send_streaming<F>(
    backend: &LlmBackend,
    model: &str,
    messages: &[ChatMessage],
    tools: &[ChatTool],
    num_ctx: u32,
    max_tokens: u32,
    on_token: F,
) -> Result<ChatMessage, String>
where
    F: Fn(&str, StreamPhase) + Send + Sync + 'static,
{
    match backend {
        LlmBackend::Ollama => {
            // Ollama streaming
            chat_send_ollama_streaming(model, messages, tools, num_ctx, max_tokens, on_token).await
        }
        LlmBackend::Mlx => {
            // MLX LM server supports native Qwen3 tool calling via its built-in
            // qwen3_coder parser. Pass tools natively — the server handles
            // <tool_call> parsing and returns structured tool_calls in the response.
            chat_send_openai_compat_streaming(
                backend.base_url(),
                model,
                messages,
                tools,
                max_tokens,
                on_token,
            )
            .await
        }
    }
}

/// Ollama streaming: POST /api/chat with stream:true
async fn chat_send_ollama_streaming<F>(
    model: &str,
    messages: &[ChatMessage],
    tools: &[ChatTool],
    num_ctx: u32,
    max_tokens: u32,
    on_token: F,
) -> Result<ChatMessage, String>
where
    F: Fn(&str, StreamPhase) + Send + Sync + 'static,
{
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;

    let msgs: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            let mut msg = serde_json::json!({ "role": m.role, "content": m.content });
            if let Some(ref tc) = m.tool_calls {
                msg["tool_calls"] = serde_json::to_value(tc).unwrap_or_default();
            }
            msg
        })
        .collect();

    let mut body = serde_json::json!({
        "model": model,
        "messages": msgs,
        "stream": true,
        "options": {
            "temperature": 0.7,
            "top_p": 0.8,
            "top_k": 20,
            "num_predict": max_tokens,
            "num_ctx": num_ctx
        }
    });

    if !tools.is_empty() {
        body["tools"] = serde_json::to_value(tools).unwrap_or_default();
    }

    let resp = client
        .post("http://localhost:11434/api/chat")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Ollama request failed: {e}"))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Ollama error: {text}"));
    }

    let mut full_content = String::new();
    let mut tool_calls: Option<Vec<ToolCallResponse>> = None;
    let mut stream = resp.bytes_stream();

    use futures::StreamExt;
    let mut buffer = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Stream error: {e}"))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // Ollama streams one JSON object per line
        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if line.trim().is_empty() {
                continue;
            }

            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) {
                if let Some(msg) = obj.get("message") {
                    if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                        if !content.is_empty() {
                            on_token(content, StreamPhase::Content);
                            full_content.push_str(content);
                        }
                    }
                    // Check for tool_calls in the final message
                    if let Some(tc) = msg.get("tool_calls") {
                        if let Ok(calls) =
                            serde_json::from_value::<Vec<ToolCallResponse>>(tc.clone())
                        {
                            if !calls.is_empty() {
                                tool_calls = Some(calls);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(ChatMessage {
        role: "assistant".into(),
        content: full_content,
        tool_calls,
        mentioned_mods: None,
    })
}

/// OpenAI-compatible streaming: POST /v1/chat/completions with stream:true (SSE)
async fn chat_send_openai_compat_streaming<F>(
    base_url: &str,
    model: &str,
    messages: &[ChatMessage],
    tools: &[ChatTool],
    max_tokens: u32,
    on_token: F,
) -> Result<ChatMessage, String>
where
    F: Fn(&str, StreamPhase) + Send + Sync + 'static,
{
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;

    let msgs: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            let mut msg = serde_json::json!({ "role": m.role, "content": m.content });
            if let Some(ref tc) = m.tool_calls {
                let oai_calls: Vec<serde_json::Value> = tc.iter().enumerate().map(|(i, c)| {
                    serde_json::json!({
                        "id": format!("call_{i}"),
                        "type": "function",
                        "function": { "name": c.function.name, "arguments": c.function.arguments.to_string() }
                    })
                }).collect();
                msg["tool_calls"] = serde_json::json!(oai_calls);
            }
            msg
        })
        .collect();

    let mut body = serde_json::json!({
        "model": model,
        "messages": msgs,
        "temperature": 0.7,
        "top_p": 0.8,
        "max_tokens": max_tokens,
        "stream": true,
        // Disable Qwen3.5 thinking/reasoning mode — it consumes all tokens on
        // internal reasoning and returns empty content. We want direct answers.
        "chat_template_kwargs": { "enable_thinking": false },
    });

    if !tools.is_empty() {
        let oai_tools: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.function.name,
                        "description": t.function.description,
                        "parameters": t.function.parameters,
                    }
                })
            })
            .collect();
        body["tools"] = serde_json::json!(oai_tools);
    }

    let resp = client
        .post(format!("{base_url}/v1/chat/completions"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("MLX request failed: {e}"))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("MLX error: {text}"));
    }

    let mut full_content = String::new();
    let mut tool_calls_map: std::collections::HashMap<usize, (String, String)> =
        std::collections::HashMap::new();
    let mut stream = resp.bytes_stream();

    use futures::StreamExt;
    let mut buffer = String::new();
    let mut chunk_count: usize = 0;
    let mut _parse_errors: usize = 0;

    // Debug file logger for streaming
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Stream error: {e}"))?;
        let chunk_str = String::from_utf8_lossy(&chunk);
        if chunk_count == 0 {
            log::debug!(
                "[CHAT] SSE first chunk: {:?}",
                &chunk_str[..chunk_str.len().min(200)]
            );
        }
        chunk_count += 1;
        buffer.push_str(&chunk_str);

        // SSE format: "data: {json}\n\n"
        while let Some(data_end) = buffer.find("\n\n") {
            let block = buffer[..data_end].to_string();
            buffer = buffer[data_end + 2..].to_string();

            for line in block.lines() {
                let line = line.trim();
                if line == "data: [DONE]" {
                    continue;
                }
                if line.starts_with(':') {
                    continue;
                } // SSE comment (keepalive)
                if let Some(json_str) = line.strip_prefix("data: ") {
                    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(json_str) {
                        if let Some(choice) = obj.get("choices").and_then(|c| c.get(0)) {
                            if let Some(delta) = choice.get("delta") {
                                // Reasoning/thinking token (Qwen3.5 etc.)
                                if let Some(reasoning) =
                                    delta.get("reasoning").and_then(|r| r.as_str())
                                {
                                    if !reasoning.is_empty() {
                                        on_token(reasoning, StreamPhase::Thinking);
                                    }
                                }
                                // Content token
                                if let Some(content) = delta.get("content").and_then(|c| c.as_str())
                                {
                                    if !content.is_empty() {
                                        on_token(content, StreamPhase::Content);
                                        full_content.push_str(content);
                                    }
                                }
                                // Tool call chunks
                                if let Some(tc_arr) =
                                    delta.get("tool_calls").and_then(|t| t.as_array())
                                {
                                    for tc in tc_arr {
                                        let idx =
                                            tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0)
                                                as usize;
                                        let entry = tool_calls_map
                                            .entry(idx)
                                            .or_insert_with(|| (String::new(), String::new()));
                                        if let Some(func) = tc.get("function") {
                                            if let Some(name) =
                                                func.get("name").and_then(|n| n.as_str())
                                            {
                                                entry.0.push_str(name);
                                            }
                                            if let Some(args) =
                                                func.get("arguments").and_then(|a| a.as_str())
                                            {
                                                entry.1.push_str(args);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    log::debug!(
        "[CHAT] SSE stream done: chunks={} content_len={} buf_remaining={}",
        chunk_count,
        full_content.len(),
        buffer.len()
    );

    // Build tool_calls from accumulated map
    let tool_calls = if tool_calls_map.is_empty() {
        None
    } else {
        let mut calls: Vec<(usize, ToolCallResponse)> = tool_calls_map
            .into_iter()
            .map(|(idx, (name, args))| {
                let arguments = serde_json::from_str(&args).unwrap_or(serde_json::json!({}));
                (
                    idx,
                    ToolCallResponse {
                        function: ToolCallFunction { name, arguments },
                    },
                )
            })
            .collect();
        calls.sort_by_key(|(idx, _)| *idx);
        Some(calls.into_iter().map(|(_, tc)| tc).collect())
    };

    Ok(ChatMessage {
        role: "assistant".into(),
        content: full_content,
        tool_calls,
        mentioned_mods: None,
    })
}

/// Ollama native API: POST /api/chat
async fn chat_send_ollama(
    model: &str,
    messages: &[ChatMessage],
    tools: &[ChatTool],
    num_ctx: u32,
    max_tokens: u32,
) -> Result<ChatMessage, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let msgs: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            let mut msg = serde_json::json!({ "role": m.role, "content": m.content });
            if let Some(ref tc) = m.tool_calls {
                msg["tool_calls"] = serde_json::to_value(tc).unwrap_or_default();
            }
            msg
        })
        .collect();

    let mut body = serde_json::json!({
        "model": model,
        "messages": msgs,
        "stream": false,
        "options": {
            "temperature": 0.7,
            "top_p": 0.8,
            "top_k": 20,
            "num_predict": max_tokens,
            "num_ctx": num_ctx
        }
    });

    if !tools.is_empty() {
        body["tools"] = serde_json::to_value(tools).unwrap_or_default();
    }

    let resp = client
        .post("http://localhost:11434/api/chat")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Ollama request failed: {e}"))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Ollama error: {text}"));
    }

    let resp_body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    log::debug!(
        "Ollama response: {}",
        serde_json::to_string_pretty(&resp_body).unwrap_or_default()
    );
    let message = resp_body.get("message").ok_or("No message in response")?;

    Ok(ChatMessage {
        role: message
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("assistant")
            .into(),
        content: message
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .into(),
        tool_calls: message
            .get("tool_calls")
            .and_then(|tc| serde_json::from_value::<Vec<ToolCallResponse>>(tc.clone()).ok()),
        mentioned_mods: None,
    })
}

/// OpenAI-compatible API: POST /v1/chat/completions (used by MLX LM, LM Studio, etc.)
async fn chat_send_openai_compat(
    base_url: &str,
    model: &str,
    messages: &[ChatMessage],
    tools: &[ChatTool],
    max_tokens: u32,
) -> Result<ChatMessage, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let msgs: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            let mut msg = serde_json::json!({ "role": m.role, "content": m.content });
            if let Some(ref tc) = m.tool_calls {
                // OpenAI format: tool_calls with id, type, function
                let oai_calls: Vec<serde_json::Value> = tc.iter().enumerate().map(|(i, c)| {
                    serde_json::json!({
                        "id": format!("call_{i}"),
                        "type": "function",
                        "function": { "name": c.function.name, "arguments": c.function.arguments.to_string() }
                    })
                }).collect();
                msg["tool_calls"] = serde_json::json!(oai_calls);
            }
            msg
        })
        .collect();

    let mut body = serde_json::json!({
        "model": model,
        "messages": msgs,
        "temperature": 0.7,
        "top_p": 0.8,
        "max_tokens": max_tokens,
    });

    if !tools.is_empty() {
        // Convert our tool format to OpenAI tool format
        let oai_tools: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.function.name,
                        "description": t.function.description,
                        "parameters": t.function.parameters,
                    }
                })
            })
            .collect();
        body["tools"] = serde_json::json!(oai_tools);
    }

    let resp = client
        .post(format!("{base_url}/v1/chat/completions"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("MLX request failed: {e}"))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("MLX error: {text}"));
    }

    let resp_body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    log::debug!(
        "MLX response: {}",
        serde_json::to_string_pretty(&resp_body).unwrap_or_default()
    );

    // OpenAI format: choices[0].message
    let message = resp_body
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .ok_or("No message in response")?;

    let role = message
        .get("role")
        .and_then(|r| r.as_str())
        .unwrap_or("assistant")
        .into();
    // content can be null when tool_calls are present; handle both null and missing
    let content: String = message
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .into();

    // Parse tool_calls from OpenAI format back to our format
    let tool_calls = message.get("tool_calls").and_then(|tc| {
        let arr = tc.as_array()?;
        let calls: Vec<ToolCallResponse> = arr
            .iter()
            .filter_map(|call| {
                let func = call.get("function")?;
                let name = func.get("name")?.as_str()?.to_string();
                let args_str = func.get("arguments")?.as_str().unwrap_or("{}");
                let arguments = serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
                Some(ToolCallResponse {
                    function: ToolCallFunction { name, arguments },
                })
            })
            .collect();
        if calls.is_empty() {
            None
        } else {
            Some(calls)
        }
    });

    Ok(ChatMessage {
        role,
        content,
        tool_calls,
        mentioned_mods: None,
    })
}

/// Load a model into memory.
pub async fn load_model(backend: &LlmBackend, model: &str, num_ctx: u32) -> Result<(), String> {
    match backend {
        LlmBackend::Ollama => {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .map_err(|e| e.to_string())?;

            let resp = client
                .post("http://localhost:11434/api/chat")
                .json(&serde_json::json!({
                    "model": model,
                    "messages": [{"role": "user", "content": "hi"}],
                    "stream": false,
                    "keep_alive": "5m",
                    "options": { "num_predict": 1, "num_ctx": num_ctx }
                }))
                .send()
                .await
                .map_err(|e| format!("Failed to load model: {e}"))?;

            if !resp.status().is_success() {
                let text = resp.text().await.unwrap_or_default();
                return Err(format!("Failed to load model: {text}"));
            }
            Ok(())
        }
        LlmBackend::Mlx => {
            // MLX LM server loads the model on first request.
            // Just verify the server is reachable.
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .map_err(|e| e.to_string())?;

            match client.get("http://localhost:8080/v1/models").send().await {
                Ok(resp) if resp.status().is_success() => Ok(()),
                _ => Err("MLX LM server not reachable at localhost:8080. Start it with: mlx_lm.server --model <model>".into()),
            }
        }
    }
}

/// Unload a model from memory.
pub async fn unload_model(backend: &LlmBackend, model: &str) -> Result<(), String> {
    match backend {
        LlmBackend::Ollama => {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| e.to_string())?;

            let _ = client
                .post("http://localhost:11434/api/chat")
                .json(&serde_json::json!({
                    "model": model,
                    "messages": [],
                    "stream": false,
                    "keep_alive": 0
                }))
                .send()
                .await;
            Ok(())
        }
        LlmBackend::Mlx => {
            // Kill the MLX LM server process to free VRAM/unified memory.
            // Find and kill all mlx_lm.server processes.
            let output = std::process::Command::new("pkill")
                .args(["-f", "mlx_lm.server"])
                .output();
            match output {
                Ok(o) if o.status.success() => {
                    log::info!("Killed MLX LM server process");
                }
                Ok(o) => {
                    log::warn!(
                        "pkill mlx_lm.server exited with {}: {}",
                        o.status,
                        String::from_utf8_lossy(&o.stderr)
                    );
                }
                Err(e) => {
                    log::warn!("Failed to run pkill: {e}");
                }
            }
            // Give it a moment to release memory
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok(())
        }
    }
}

/// Path to the Corkscrew MLX virtual environment.
fn mlx_venv_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".corkscrew")
        .join("mlx-venv")
}

/// Path to the Python binary inside the MLX venv.
fn mlx_python() -> std::path::PathBuf {
    mlx_venv_dir().join("bin").join("python3")
}

/// Check if MLX LM is available (venv exists and package installed).
pub async fn check_mlx_status() -> bool {
    #[cfg(target_os = "macos")]
    {
        let python = mlx_python();
        if !python.exists() {
            return false;
        }
        match tokio::process::Command::new(&python)
            .args(["-c", "import mlx_lm; print('ok')"])
            .output()
            .await
        {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Install MLX LM into a dedicated venv.
pub async fn install_mlx() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let venv_dir = mlx_venv_dir();

        // Create venv if it doesn't exist
        if !venv_dir.exists() {
            let output = tokio::process::Command::new("python3")
                .args(["-m", "venv", &venv_dir.to_string_lossy()])
                .output()
                .await
                .map_err(|e| format!("Failed to create venv: {e}"))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("Failed to create venv: {stderr}"));
            }
        }

        // Install mlx-lm into the venv
        let python = mlx_python();
        let output = tokio::process::Command::new(&python)
            .args(["-m", "pip", "install", "--upgrade", "mlx-lm"])
            .output()
            .await
            .map_err(|e| format!("Failed to run pip: {e}"))?;

        if output.status.success() {
            Ok("MLX LM installed successfully.".into())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("Install failed: {stderr}"))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("MLX is only available on macOS with Apple Silicon.".into())
    }
}

/// Start the MLX LM server for a given model.
/// If a server is already running, checks that our model is loaded.
/// If an external server with wrong config is detected, kills and restarts.
pub async fn start_mlx_server(model: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| e.to_string())?;

        // Check if already running with our model
        if let Ok(resp) = client.get("http://localhost:8080/v1/models").send().await {
            if resp.status().is_success() {
                if let Ok(body) = resp.text().await {
                    if body.contains(model) {
                        log::info!("MLX server already running with model {model}");
                        return Ok(());
                    }
                    // Server running but wrong model — kill it and restart
                    log::info!("MLX server running with different model, restarting for {model}");
                    let _ = std::process::Command::new("pkill")
                        .args(["-f", "mlx_lm.server"])
                        .output();
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }

        // Start the server using the venv Python
        let python = mlx_python();
        tokio::process::Command::new(&python)
            .args([
                "-m",
                "mlx_lm.server",
                "--model",
                model,
                "--port",
                "8080",
                "--max-tokens",
                "4096",
                "--chat-template-args",
                r#"{"enable_thinking":false}"#,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to start MLX LM server: {e}"))?;

        // Wait for it to be ready
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Ok(resp) = client.get("http://localhost:8080/v1/models").send().await {
                if resp.status().is_success() {
                    return Ok(());
                }
            }
        }

        Err("MLX LM server started but not responding. Model may still be loading.".into())
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err("MLX is only available on macOS with Apple Silicon.".into())
    }
}
