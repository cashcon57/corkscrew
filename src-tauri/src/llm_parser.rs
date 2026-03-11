//! Tier 2: LLM-based instruction parser (local Ollama + cloud backends).
//!
//! Handles instructions that the deterministic parser could not parse.
//! Supports both local models (via Ollama) and free cloud APIs.

use serde::{Deserialize, Serialize};

use crate::instruction_types::*;

// ---------------------------------------------------------------------------
// System prompt — shared across all LLM backends
// ---------------------------------------------------------------------------

/// Build the system prompt for instruction parsing.
///
/// This prompt is carefully designed for maximum determinism:
/// - Explicit JSON schema with zero ambiguity
/// - Grounding: only mod names from the provided list are valid
/// - Few-shot examples covering common patterns
/// - Negative examples showing what NOT to output
/// - No chain-of-thought — direct structured output only
pub fn build_system_prompt(
    available_mods: &[String],
    platform: &str,
    game_version: &str,
) -> String {
    let mod_list = if available_mods.is_empty() {
        "No mod list available — use the exact mod names from the instructions.".to_string()
    } else {
        let mods: Vec<String> = available_mods.iter().take(200).cloned().collect();
        format!(
            "Available mods (use ONLY these exact names):\n{}",
            mods.join("\n")
        )
    };

    format!(
        r#"You are a mod collection instruction parser. Your ONLY job is to convert natural language collection author instructions into a JSON array of structured actions.

## RULES — FOLLOW EXACTLY
1. Output ONLY a JSON array. No explanation, no markdown, no preamble.
2. Each element must match the schema below EXACTLY.
3. For mod names, use ONLY names from the "Available mods" list. If a mod name in the instructions does not match any available mod, skip that instruction.
4. If you cannot parse an instruction, output {{"type": "manual_step", "description": "<original text>"}} instead of guessing.
5. Never invent mod names, plugin names, or settings that are not mentioned in the instructions.
6. Set "confidence" to 0.9 for clear instructions, 0.7 for ambiguous ones, 0.5 if you're unsure.

## CURRENT ENVIRONMENT
- Platform: {platform}
- Game version: {game_version}

## {mod_list}

## ACTION SCHEMA
Each action is a JSON object with these fields:

```json
{{
  "type": "enable_mod" | "disable_mod" | "enable_all_optional" | "disable_all_optional" | "set_fomod_choice" | "set_ini" | "set_load_order" | "manual_step",
  "mod_name": "exact name from available mods list",
  "condition": {{
    "type": "always" | "game_version" | "platform" | "dlc_present" | "mod_installed",
    "value": "ae" | "se" | "wine" | "proton" | "wine_or_proton" | "native" | "<dlc/mod name>"
  }},
  "confidence": 0.5 to 1.0,
  "source_text": "the original instruction line"
}}
```

Extra fields per type:
- `set_fomod_choice`: add `"option": "choice name"`, `"step": "step name"` (optional), `"group": "group name"` (optional)
- `set_ini`: add `"file": "Skyrim.ini"`, `"section": "General"`, `"key": "bFoo"`, `"value": "1"`
- `set_load_order`: add `"plugin": "name.esp"`, `"position": {{"type": "after"|"before"|"bottom"|"top", "reference": "other.esp"}}`
- `manual_step`: add `"description": "what to do"`, `"url": "https://..." (optional)`

## FEW-SHOT EXAMPLES

Input: "Enable all optional mods if you have AE"
Output:
```json
[{{"type": "enable_all_optional", "condition": {{"type": "game_version", "value": "ae"}}, "confidence": 0.95, "source_text": "Enable all optional mods if you have AE"}}]
```

Input: "Disable Community Shaders if on Wine or Proton"
Output:
```json
[{{"type": "disable_mod", "mod_name": "Community Shaders", "condition": {{"type": "platform", "value": "wine_or_proton"}}, "confidence": 0.95, "source_text": "Disable Community Shaders if on Wine or Proton"}}]
```

Input: "Choose the 'Performance' preset during the FOMOD for ENB"
Output:
```json
[{{"type": "set_fomod_choice", "mod_name": "ENB", "option": "Performance", "condition": {{"type": "always"}}, "confidence": 0.9, "source_text": "Choose the 'Performance' preset during the FOMOD for ENB"}}]
```

Input: "This collection is designed for a heavily modded experience"
Output:
```json
[]
```
(This is informational, not an actionable instruction — output an empty array.)

## DO NOT
- Do NOT output markdown code fences in your response
- Do NOT explain your reasoning
- Do NOT output anything other than a JSON array
- Do NOT guess mod names that aren't in the available list
- Do NOT create actions for purely informational text (descriptions, credits, thank-yous)
"#
    )
}

// ---------------------------------------------------------------------------
// LLM tool definitions for chat mode
// ---------------------------------------------------------------------------

/// Get the tool definitions available to the LLM, filtered by capability tier.
pub fn get_tools_for_tier(tier: &ModelCapabilityTier) -> Vec<LlmTool> {
    let mut tools = Vec::new();

    // Basic tier: read-only queries + enable/disable
    tools.push(LlmTool {
        name: "list_mods".into(),
        description: "List all installed mods with their enabled/disabled status.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
    });

    tools.push(LlmTool {
        name: "enable_mod".into(),
        description: "Enable a mod by name.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "mod_name": { "type": "string", "description": "Exact mod name" }
            },
            "required": ["mod_name"]
        }),
    });

    tools.push(LlmTool {
        name: "disable_mod".into(),
        description: "Disable a mod by name.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "mod_name": { "type": "string", "description": "Exact mod name" }
            },
            "required": ["mod_name"]
        }),
    });

    tools.push(LlmTool {
        name: "get_mod_info".into(),
        description: "Get detailed information about a specific mod.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "mod_name": { "type": "string", "description": "Mod name to look up" }
            },
            "required": ["mod_name"]
        }),
    });

    if *tier < ModelCapabilityTier::Standard {
        return tools;
    }

    // Standard tier: + FOMOD, INI edits
    tools.push(LlmTool {
        name: "set_ini_setting".into(),
        description: "Set a value in a game INI file.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "file": { "type": "string", "description": "INI filename (e.g., Skyrim.ini)" },
                "section": { "type": "string", "description": "INI section (e.g., General)" },
                "key": { "type": "string", "description": "Setting key" },
                "value": { "type": "string", "description": "Setting value" }
            },
            "required": ["file", "section", "key", "value"]
        }),
    });

    tools.push(LlmTool {
        name: "get_fomod_choices".into(),
        description: "Get available FOMOD installer choices for a mod.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "mod_name": { "type": "string", "description": "Mod name" }
            },
            "required": ["mod_name"]
        }),
    });

    if *tier < ModelCapabilityTier::Advanced {
        return tools;
    }

    // Advanced tier: + load order, conflict resolution
    tools.push(LlmTool {
        name: "get_load_order".into(),
        description: "Get the current plugin load order.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
    });

    tools.push(LlmTool {
        name: "move_plugin".into(),
        description: "Move a plugin in the load order.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "plugin": { "type": "string", "description": "Plugin filename (e.g., mod.esp)" },
                "position": { "type": "string", "enum": ["after", "before", "bottom", "top"] },
                "reference": { "type": "string", "description": "Reference plugin (for after/before)" }
            },
            "required": ["plugin", "position"]
        }),
    });

    tools.push(LlmTool {
        name: "get_conflicts".into(),
        description: "Get file conflicts between installed mods.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
    });

    tools
}

// ---------------------------------------------------------------------------
// Ollama backend
// ---------------------------------------------------------------------------

/// Check if Ollama is running locally.
pub async fn check_ollama_status() -> OllamaStatus {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    let running = match client.get("http://localhost:11434/api/tags").send().await {
        Ok(resp) if resp.status().is_success() => true,
        _ => false,
    };

    let mut installed_models = Vec::new();
    if running {
        if let Ok(resp) = client.get("http://localhost:11434/api/tags").send().await {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if let Some(models) = body.get("models").and_then(|m| m.as_array()) {
                    for model in models {
                        if let Some(name) = model.get("name").and_then(|n| n.as_str()) {
                            let size = model.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                            installed_models.push(OllamaModel {
                                name: name.to_string(),
                                size_bytes: size,
                                size_display: format_bytes(size),
                                description: String::new(),
                                expected_accuracy: 0.0,
                                supports_tool_use: false,
                                min_memory_bytes: 0,
                            });
                        }
                    }
                }
            }
        }
    }

    // Check if ollama binary exists
    let installed = running || which_ollama().is_some();

    OllamaStatus {
        installed,
        running,
        available_models: installed_models,
    }
}

fn which_ollama() -> Option<String> {
    std::process::Command::new("which")
        .arg("ollama")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.0} MB", bytes as f64 / 1_000_000.0)
    } else {
        format!("{} bytes", bytes)
    }
}

/// Pull (download) a model via Ollama. Returns progress updates.
pub async fn pull_ollama_model(model_name: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    let resp = client
        .post("http://localhost:11434/api/pull")
        .json(&serde_json::json!({ "name": model_name, "stream": false }))
        .send()
        .await
        .map_err(|e| format!("Failed to contact Ollama: {e}"))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Ollama pull failed: {body}"));
    }

    Ok(())
}

/// Delete a model from Ollama.
pub async fn delete_ollama_model(model_name: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    let resp = client
        .delete("http://localhost:11434/api/delete")
        .json(&serde_json::json!({ "name": model_name }))
        .send()
        .await
        .map_err(|e| format!("Failed to contact Ollama: {e}"))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Ollama delete failed: {body}"));
    }

    Ok(())
}

/// Unload model from memory (keep on disk).
pub async fn unload_ollama_model(model_name: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    // Setting keep_alive to 0 unloads the model immediately
    let _ = client
        .post("http://localhost:11434/api/generate")
        .json(&serde_json::json!({
            "model": model_name,
            "keep_alive": 0
        }))
        .send()
        .await;
    Ok(())
}

/// Parse instructions using a local Ollama model.
pub async fn parse_with_ollama(
    model: &str,
    instructions: &str,
    available_mods: &[String],
    platform: &str,
    game_version: &str,
) -> Result<Vec<ConditionalAction>, String> {
    let system = build_system_prompt(available_mods, platform, game_version);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .post("http://localhost:11434/api/chat")
        .json(&serde_json::json!({
            "model": model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": instructions }
            ],
            "stream": false,
            "format": {
                "type": "object",
                "properties": {
                    "actions": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "type": { "type": "string", "enum": [
                                    "enable_mod", "disable_mod", "enable_all_optional",
                                    "disable_all_optional", "set_fomod_choice", "set_ini_setting",
                                    "set_load_order", "manual_step"
                                ]},
                                "mod_name": { "type": "string" },
                                "description": { "type": "string" },
                                "file": { "type": "string" },
                                "section": { "type": "string" },
                                "key": { "type": "string" },
                                "value": { "type": "string" },
                                "plugin": { "type": "string" },
                                "option": { "type": "string" },
                                "step": { "type": "string" },
                                "group": { "type": "string" },
                                "position": { "type": "string" },
                                "url": { "type": "string" },
                                "condition": { "type": "object" }
                            },
                            "required": ["type"]
                        }
                    }
                },
                "required": ["actions"]
            },
            "options": {
                "temperature": 0.0,
                "top_k": 1,
                "top_p": 1.0,
                "seed": 42,
                "num_predict": 4096,
                "repeat_penalty": 1.0
            }
        }))
        .send()
        .await
        .map_err(|e| format!("Ollama request failed: {e}"))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Ollama returned error: {body}"));
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let content = body
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("[]");

    parse_llm_response(content, model)
}

// ---------------------------------------------------------------------------
// Cloud LLM backends
// ---------------------------------------------------------------------------

/// Cloud provider configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CloudProvider {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub requires_api_key: bool,
    pub free_tier_info: String,
}

pub fn available_cloud_providers() -> Vec<CloudProvider> {
    vec![
        CloudProvider {
            name: "gemini".into(),
            display_name: "Google Gemini".into(),
            description: "Gemini 2.5 Flash. Best free tier, excellent structured output.".into(),
            requires_api_key: true,
            free_tier_info: "Free: 10 req/min, 250 req/day, 250K tokens/min".into(),
        },
        CloudProvider {
            name: "cerebras".into(),
            display_name: "Cerebras".into(),
            description: "Llama 3.3 70B. Ultra-fast wafer-scale inference. OpenAI-compatible API."
                .into(),
            requires_api_key: true,
            free_tier_info: "Free: 1M tokens/day, 30 req/min. No credit card.".into(),
        },
        CloudProvider {
            name: "groq".into(),
            display_name: "Groq".into(),
            description: "Llama 3.3 70B. Sub-second inference, strict JSON schema enforcement."
                .into(),
            requires_api_key: true,
            free_tier_info: "Free: 30 req/min, ~500K tokens/day. No credit card.".into(),
        },
    ]
}

/// Parse instructions using Groq's free API.
pub async fn parse_with_groq(
    api_key: &str,
    instructions: &str,
    available_mods: &[String],
    platform: &str,
    game_version: &str,
) -> Result<Vec<ConditionalAction>, String> {
    let system = build_system_prompt(available_mods, platform, game_version);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": "llama-3.3-70b-versatile",
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": instructions }
            ],
            "temperature": 0.0,
            "max_tokens": 4096,
            "response_format": { "type": "json_object" },
            "seed": 42
        }))
        .send()
        .await
        .map_err(|e| format!("Groq request failed: {e}"))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Groq returned error: {body}"));
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let content = body
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .unwrap_or("[]");

    parse_llm_response(content, "groq:llama-3.3-70b")
}

/// Parse instructions using Google Gemini's free API.
pub async fn parse_with_gemini(
    api_key: &str,
    instructions: &str,
    available_mods: &[String],
    platform: &str,
    game_version: &str,
) -> Result<Vec<ConditionalAction>, String> {
    let system = build_system_prompt(available_mods, platform, game_version);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .post(format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent?key={api_key}"
        ))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "contents": [
                { "role": "user", "parts": [{ "text": format!("{system}\n\n---\n\nParse these instructions:\n{instructions}") }] }
            ],
            "generationConfig": {
                "temperature": 0.0,
                "topP": 1.0,
                "maxOutputTokens": 4096,
                "responseMimeType": "application/json"
            }
        }))
        .send()
        .await
        .map_err(|e| format!("Gemini request failed: {e}"))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Gemini returned error: {body}"));
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let content = body
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(|c| c.as_str())
        .unwrap_or("[]");

    parse_llm_response(content, "gemini:2.5-flash")
}

/// Parse instructions using Cerebras free API (OpenAI-compatible).
pub async fn parse_with_cerebras(
    api_key: &str,
    instructions: &str,
    available_mods: &[String],
    platform: &str,
    game_version: &str,
) -> Result<Vec<ConditionalAction>, String> {
    let system = build_system_prompt(available_mods, platform, game_version);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .post("https://api.cerebras.ai/v1/chat/completions")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": "llama-3.3-70b",
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": instructions }
            ],
            "temperature": 0.0,
            "max_tokens": 4096,
            "response_format": { "type": "json_object" },
            "seed": 42
        }))
        .send()
        .await
        .map_err(|e| format!("Cerebras request failed: {e}"))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Cerebras returned error: {body}"));
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let content = body
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .unwrap_or("[]");

    parse_llm_response(content, "cerebras:llama-3.3-70b")
}

// ---------------------------------------------------------------------------
// Response parsing & validation
// ---------------------------------------------------------------------------

/// Parse raw LLM JSON response into validated ConditionalActions.
fn parse_llm_response(raw: &str, model: &str) -> Result<Vec<ConditionalAction>, String> {
    // Strip any markdown code fences the model might have added
    let cleaned = raw
        .trim()
        .strip_prefix("```json")
        .or_else(|| raw.trim().strip_prefix("```"))
        .unwrap_or(raw.trim())
        .strip_suffix("```")
        .unwrap_or(raw.trim())
        .trim();

    // Try parsing as a JSON array directly
    let items: Vec<serde_json::Value> = if cleaned.starts_with('[') {
        serde_json::from_str(cleaned).map_err(|e| format!("Invalid JSON from {model}: {e}"))?
    } else if cleaned.starts_with('{') {
        // Some models wrap in an object like {"actions": [...]}
        let obj: serde_json::Value =
            serde_json::from_str(cleaned).map_err(|e| format!("Invalid JSON from {model}: {e}"))?;
        if let Some(actions) = obj.get("actions").and_then(|a| a.as_array()) {
            actions.clone()
        } else {
            // Single action object
            vec![obj]
        }
    } else {
        return Err(format!(
            "LLM output is not JSON: {}",
            &cleaned[..cleaned.len().min(100)]
        ));
    };

    let mut actions = Vec::new();
    for item in items {
        if let Some(action) = parse_single_action(&item) {
            actions.push(action);
        }
    }

    Ok(actions)
}

/// Parse a single JSON action object into a ConditionalAction.
fn parse_single_action(item: &serde_json::Value) -> Option<ConditionalAction> {
    let action_type = item.get("type")?.as_str()?;
    let source_text = item
        .get("source_text")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let confidence = item
        .get("confidence")
        .and_then(|c| c.as_f64())
        .unwrap_or(0.7) as f32;

    let condition = parse_condition(item.get("condition"));

    let action = match action_type {
        "enable_mod" => {
            let mod_name = item.get("mod_name")?.as_str()?.to_string();
            InstructionAction::EnableMod { mod_name }
        }
        "disable_mod" => {
            let mod_name = item.get("mod_name")?.as_str()?.to_string();
            InstructionAction::DisableMod { mod_name }
        }
        "enable_all_optional" => InstructionAction::EnableAllOptional,
        "disable_all_optional" => InstructionAction::DisableAllOptional,
        "set_fomod_choice" => {
            let mod_name = item.get("mod_name")?.as_str()?.to_string();
            let option = item.get("option")?.as_str()?.to_string();
            let step = item.get("step").and_then(|s| s.as_str()).map(String::from);
            let group = item.get("group").and_then(|s| s.as_str()).map(String::from);
            InstructionAction::SetFomodChoice {
                mod_name,
                step,
                group,
                option,
            }
        }
        "set_ini" => {
            let file = item.get("file")?.as_str()?.to_string();
            let section = item.get("section")?.as_str()?.to_string();
            let key = item.get("key")?.as_str()?.to_string();
            let value = item.get("value")?.as_str()?.to_string();
            InstructionAction::SetIniSetting {
                file,
                section,
                key,
                value,
            }
        }
        "set_load_order" => {
            let plugin = item.get("plugin")?.as_str()?.to_string();
            let pos = item.get("position")?;
            let position = if let Some(pos_str) = pos.as_str() {
                match pos_str {
                    "bottom" => LoadOrderPosition::Bottom,
                    "top" => LoadOrderPosition::Top,
                    _ => return None,
                }
            } else {
                let pos_type = pos.get("type")?.as_str()?;
                match pos_type {
                    "after" => LoadOrderPosition::After {
                        reference: pos.get("reference")?.as_str()?.to_string(),
                    },
                    "before" => LoadOrderPosition::Before {
                        reference: pos.get("reference")?.as_str()?.to_string(),
                    },
                    "bottom" => LoadOrderPosition::Bottom,
                    "top" => LoadOrderPosition::Top,
                    _ => return None,
                }
            };
            InstructionAction::SetLoadOrder { plugin, position }
        }
        "manual_step" => {
            let description = item
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("Manual action required")
                .to_string();
            let url = item.get("url").and_then(|u| u.as_str()).map(String::from);
            InstructionAction::ManualStep { description, url }
        }
        _ => return None,
    };

    Some(ConditionalAction {
        action,
        condition,
        source_text,
        confidence,
    })
}

fn parse_condition(val: Option<&serde_json::Value>) -> InstructionCondition {
    let val = match val {
        Some(v) if v.is_object() => v,
        _ => return InstructionCondition::Always,
    };

    let cond_type = match val.get("type").and_then(|t| t.as_str()) {
        Some(t) => t,
        None => return InstructionCondition::Always,
    };

    let value = val.get("value").and_then(|v| v.as_str()).unwrap_or("");

    match cond_type {
        "always" => InstructionCondition::Always,
        "game_version" => {
            let version = match value.to_lowercase().as_str() {
                "ae" | "anniversary" => GameVersionMatch::Ae,
                "se" | "special" => GameVersionMatch::Se,
                other => GameVersionMatch::Pattern(other.to_string()),
            };
            InstructionCondition::GameVersion { version }
        }
        "platform" => {
            let platform = match value.to_lowercase().as_str() {
                "wine" => PlatformMatch::Wine,
                "proton" => PlatformMatch::Proton,
                "wine_or_proton" | "wineorproton" => PlatformMatch::WineOrProton,
                "native" | "windows" => PlatformMatch::Native,
                _ => PlatformMatch::WineOrProton,
            };
            InstructionCondition::Platform { platform }
        }
        "dlc_present" => InstructionCondition::DlcPresent {
            dlc_name: value.to_string(),
        },
        "mod_installed" => InstructionCondition::ModInstalled {
            mod_name: value.to_string(),
        },
        _ => InstructionCondition::Always,
    }
}
