//! Shared types for the collection instruction parsing system.
//!
//! Instructions flow through three tiers:
//! 1. Deterministic regex/pattern parser (always runs first)
//! 2. LLM parser (local or cloud, user opt-in)
//! 3. Manual mode (user reads instructions and picks actions)

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Actions — what to do
// ---------------------------------------------------------------------------

/// A single action parsed from collection instructions.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InstructionAction {
    /// Enable a specific mod by name.
    EnableMod { mod_name: String },
    /// Disable a specific mod by name.
    DisableMod { mod_name: String },
    /// Enable all optional mods in the collection.
    EnableAllOptional,
    /// Disable all optional mods in the collection.
    DisableAllOptional,
    /// Set a FOMOD installer choice for a mod.
    SetFomodChoice {
        mod_name: String,
        /// Which step in the FOMOD installer (human label or index).
        step: Option<String>,
        /// Which group/category within that step.
        group: Option<String>,
        /// The option to select.
        option: String,
    },
    /// Set an INI configuration value.
    SetIniSetting {
        file: String,
        section: String,
        key: String,
        value: String,
    },
    /// Adjust load order for a plugin.
    SetLoadOrder {
        plugin: String,
        position: LoadOrderPosition,
    },
    /// A step that must be done manually (download external file, etc.).
    ManualStep {
        description: String,
        url: Option<String>,
    },
}

/// Where to place a plugin in the load order.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LoadOrderPosition {
    After { reference: String },
    Before { reference: String },
    Bottom,
    Top,
}

// ---------------------------------------------------------------------------
// Conditions — when to apply an action
// ---------------------------------------------------------------------------

/// A condition that gates when an action should be applied.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InstructionCondition {
    /// Always apply this action.
    Always,
    /// Only for a specific game version.
    GameVersion { version: GameVersionMatch },
    /// Only on a specific platform.
    Platform { platform: PlatformMatch },
    /// Only if a specific DLC is present.
    DlcPresent { dlc_name: String },
    /// Only if another mod is installed.
    ModInstalled { mod_name: String },
    /// Only if another mod is NOT installed.
    ModNotInstalled { mod_name: String },
    /// Multiple conditions that all must be true.
    All {
        conditions: Vec<InstructionCondition>,
    },
    /// Multiple conditions where at least one must be true.
    Any {
        conditions: Vec<InstructionCondition>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GameVersionMatch {
    /// Anniversary Edition (1.6.x)
    Ae,
    /// Special Edition (1.5.x)
    Se,
    /// A specific version string pattern.
    Pattern(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PlatformMatch {
    Wine,
    Proton,
    WineOrProton,
    Native,
}

// ---------------------------------------------------------------------------
// Parsed result
// ---------------------------------------------------------------------------

/// A single conditional action parsed from instructions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConditionalAction {
    /// The action to perform.
    pub action: InstructionAction,
    /// When to apply this action.
    pub condition: InstructionCondition,
    /// The original instruction text this was parsed from.
    pub source_text: String,
    /// Parsing confidence (0.0–1.0). Deterministic parser always returns 1.0.
    pub confidence: f32,
}

/// Result of parsing collection instructions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParsedInstructions {
    /// Successfully parsed actions.
    pub actions: Vec<ConditionalAction>,
    /// Lines or sections that could not be parsed.
    pub unparsed_lines: Vec<String>,
    /// Where the parse came from.
    pub source: ParseSource,
    /// True if everything was parsed (no unparsed lines).
    pub fully_parsed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ParseSource {
    Deterministic,
    LocalLlm { model: String },
    CloudLlm { provider: String },
    Manual,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Result of validating a parsed action against the actual mod list / game state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ValidatedAction {
    pub action: ConditionalAction,
    pub status: ValidationStatus,
    /// The resolved mod ID if the action references a mod.
    pub resolved_mod_id: Option<i64>,
    /// Human-readable explanation of why validation passed/failed.
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    /// Action is valid and can be executed.
    Valid,
    /// Action is probably valid but needs user confirmation.
    NeedsConfirmation,
    /// Action was rejected by validation.
    Rejected,
}

// ---------------------------------------------------------------------------
// LLM configuration
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmConfig {
    /// User's preferred LLM mode.
    pub preference: LlmPreference,
    /// Selected local model identifier (e.g., "qwen2.5:7b").
    pub local_model: Option<String>,
    /// Selected cloud provider.
    pub cloud_provider: Option<String>,
    /// Cloud API key (if required by provider).
    pub cloud_api_key: Option<String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            preference: LlmPreference::None,
            local_model: None,
            cloud_provider: None,
            cloud_api_key: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LlmPreference {
    None,
    Local,
    Cloud,
}

// ---------------------------------------------------------------------------
// Ollama types
// ---------------------------------------------------------------------------

/// Status of the local Ollama installation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OllamaStatus {
    pub installed: bool,
    pub running: bool,
    pub available_models: Vec<OllamaModel>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OllamaModel {
    pub name: String,
    pub size_bytes: u64,
    /// Human-readable size (e.g., "2.3 GB").
    pub size_display: String,
    /// Brief description of accuracy/speed tradeoff.
    pub description: String,
    /// Expected accuracy for instruction parsing (0.0–1.0).
    pub expected_accuracy: f32,
    /// Whether this model supports native tool/function calling.
    pub supports_tool_use: bool,
    /// Minimum system memory (bytes) to run this model comfortably.
    #[serde(default)]
    pub min_memory_bytes: u64,
}

/// Recommended models with metadata for the UI.
/// Updated March 2026 based on BFCL-V4 leaderboard and Qwen3.5 release.
/// All models run via Ollama (llama.cpp + Metal on Apple Silicon, GGUF format).
///
/// Memory guidance (Q4_K_M quantization):
/// - 8 GB system: up to ~4B models comfortably
/// - 16 GB system: up to ~9B models
/// - 24 GB system: up to ~14B models
/// - 32 GB+: up to ~30B MoE models
pub fn recommended_models() -> Vec<OllamaModel> {
    vec![
        // OP Mode — 64 GB+ unified memory
        OllamaModel {
            name: "qwen3:32b".into(),
            size_bytes: 20_000_000_000,
            size_display: "20 GB".into(),
            description: "OP Mode. Dense 32B — best reasoning and tool use. Needs 64 GB+.".into(),
            expected_accuracy: 0.98,
            supports_tool_use: true,
            min_memory_bytes: 64_000_000_000,
        },
        OllamaModel {
            name: "qwen3:30b-a3b".into(),
            size_bytes: 18_000_000_000,
            size_display: "18 GB".into(),
            description:
                "MoE — 30B total, only 3B activated. Runs fast, thinks big. Best on Ollama.".into(),
            expected_accuracy: 0.96,
            supports_tool_use: true,
            min_memory_bytes: 32_000_000_000,
        },
        OllamaModel {
            name: "qwen3:8b".into(),
            size_bytes: 5_200_000_000,
            size_display: "5.2 GB".into(),
            description: "Strong reasoning + reliable tool calling. Best dense model for Ollama."
                .into(),
            expected_accuracy: 0.88,
            supports_tool_use: true,
            min_memory_bytes: 16_000_000_000,
        },
        OllamaModel {
            name: "qwen3:4b".into(),
            size_bytes: 2_600_000_000,
            size_display: "2.6 GB".into(),
            description: "Good balance. Reliable Hermes tool calling on Ollama.".into(),
            expected_accuracy: 0.75,
            supports_tool_use: true,
            min_memory_bytes: 8_000_000_000,
        },
        OllamaModel {
            name: "qwen3:1.7b".into(),
            size_bytes: 1_100_000_000,
            size_display: "1.1 GB".into(),
            description: "Fastest. Hermes tool calling works reliably on Ollama.".into(),
            expected_accuracy: 0.55,
            supports_tool_use: true,
            min_memory_bytes: 4_000_000_000,
        },
    ]
}

/// Returns true if the system has 64 GB+ memory (OP Mode eligible).
pub fn is_op_mode(total_memory_bytes: u64) -> bool {
    total_memory_bytes >= 64_000_000_000
}

/// Context window size. The real scaling is model size, not context.
/// 12K is enough for system prompt (~550 tok) + tools (~1800 tok) + conversation + tool results.
/// OP Mode gets 16K for slightly longer conversations, but the win is the 32B model.
pub fn context_size_for_memory(total_memory_bytes: u64) -> u32 {
    if total_memory_bytes >= 64_000_000_000 {
        16384 // OP Mode — modest bump, model quality is the real gain
    } else {
        8192
    }
}

/// Max response tokens. OP Mode models are smarter, let them speak a bit more.
pub fn max_response_tokens(num_ctx: u32) -> u32 {
    if num_ctx >= 16384 {
        1536
    } else {
        1024
    }
}

/// Pick the best model for the user's available memory.
/// Models are ordered largest-first, so return the first one that fits.
pub fn recommended_model_for_memory(total_memory_bytes: u64) -> String {
    let models = recommended_models();
    for m in &models {
        if m.min_memory_bytes <= total_memory_bytes {
            return m.name.clone();
        }
    }
    // Fallback to smallest
    models
        .last()
        .map(|m| m.name.clone())
        .unwrap_or_else(|| "qwen3:1.7b".into())
}

// ---------------------------------------------------------------------------
// Chat types (framework for future LLM chat feature)
// ---------------------------------------------------------------------------

/// A tool that the LLM can call during a chat session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmTool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// A message in an LLM chat session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    /// Tool calls made by the assistant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Result of a tool call (role = tool).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Capability tiers — less capable models get fewer tools.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapabilityTier {
    /// 1-2B models: only basic enable/disable, read-only queries.
    Basic = 0,
    /// 3-4B models: enable/disable, FOMOD choices, INI edits.
    Standard = 1,
    /// 7-8B models: full tool access including load order, conflict resolution.
    Advanced = 2,
}

impl ModelCapabilityTier {
    /// Determine capability tier from model name/size.
    pub fn from_model_name(name: &str) -> Self {
        let lower = name.to_lowercase();
        // MoE models with large total params but small active params → Advanced
        if lower.contains("a3b") || lower.contains("moe") {
            return Self::Advanced;
        }
        if lower.contains("1.5b") || lower.contains("1b") || lower.contains("2b") {
            Self::Basic
        } else if lower.contains("3b")
            || lower.contains("3.8b")
            || lower.contains("4b")
            || lower.contains("mini")
        {
            Self::Standard
        } else {
            Self::Advanced
        }
    }
}
