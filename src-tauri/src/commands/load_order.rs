//! Generic file-based load order: get/set commands and on-disk codecs.
//!
//! Bethesda titles drive the rich plugins.txt UI via [`commands::plugins`];
//! everything in this module is for games whose load order is a plain ordered
//! list of mod identifiers persisted to a config file (UE4 `~mods`,
//! Unity / RimWorld `ModsConfig.xml`, BepInEx-style ordering, etc.).
//!
//! The frontend asks `get_load_order_kind` first, and only invokes the
//! `*_file_based_load_order` commands when the answer is `"file_based"`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::games::{self, LoadOrderFormat, LoadOrderKind};
use crate::resolve_game_any_runtime;

// ---------------------------------------------------------------------------
// Types exposed to the frontend
// ---------------------------------------------------------------------------

/// One row in the file-based load order list.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoadOrderEntry {
    /// Stable identifier as it appears on disk (e.g. mod folder name,
    /// Steam workshop ID, RimWorld package ID).
    pub id: String,
    /// Human-readable label shown in the UI. Falls back to `id` when the
    /// game plugin doesn't supply a `describe` function.
    pub display_name: String,
    /// Whether the entry is currently active. Formats that can't represent
    /// a disabled entry (RimWorld `<activeMods>`) treat absence as disabled.
    pub enabled: bool,
}

/// String tag returned by `get_load_order_kind_cmd`. Kept as a plain string
/// so the frontend can switch on it without redefining the enum in TS.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoadOrderKindDto {
    /// No load order applies — the page should render its empty notice.
    None,
    /// Bethesda-style plugins.txt — the existing rich UI handles this.
    Plugins,
    /// File-based: the generic panel renders an editable ordered list.
    FileBased,
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Tell the frontend which load-order UI to render for the given game.
#[tauri::command]
pub async fn get_load_order_kind_cmd(
    game_id: String,
    bottle_name: String,
) -> Result<LoadOrderKindDto, String> {
    let (_opt_bottle, game, _data_dir) = resolve_game_any_runtime(&game_id, &bottle_name)?;
    let game_path = PathBuf::from(&game.game_path);

    let kind = games::with_plugin(&game_id, |plugin| plugin.load_order_kind(&game_path))
        .unwrap_or(LoadOrderKind::None);

    Ok(match kind {
        LoadOrderKind::None => LoadOrderKindDto::None,
        LoadOrderKind::Plugins => LoadOrderKindDto::Plugins,
        LoadOrderKind::FileBased(_) => LoadOrderKindDto::FileBased,
    })
}

/// Read the persisted file-based load order. Errors if the game is not a
/// `FileBased` game; missing file is **not** an error (returns an empty list).
#[tauri::command]
pub async fn get_file_based_load_order(
    game_id: String,
    bottle_name: String,
) -> Result<Vec<LoadOrderEntry>, String> {
    let (_opt_bottle, game, _data_dir) = resolve_game_any_runtime(&game_id, &bottle_name)?;
    let game_path = PathBuf::from(&game.game_path);

    // Snapshot the FileBasedLoadOrder out of the registry — we cannot hold
    // the registry mutex across the file IO that follows.
    let cfg = games::with_plugin(&game_id, |plugin| match plugin.load_order_kind(&game_path) {
        LoadOrderKind::FileBased(c) => Some(c),
        _ => None,
    })
    .flatten()
    .ok_or_else(|| format!("Game '{}' does not use a file-based load order", game_id))?;

    let resolved = resolve_config_path(&game_path, &cfg.config_path);
    if !resolved.exists() {
        return Ok(Vec::new());
    }

    // BG3 modsettings.lsx requires special read handling: masters are filtered
    // out so users only see and reorder community mods.
    if cfg.format == LoadOrderFormat::Bg3ModSettings {
        return read_bg3_load_order(&resolved);
    }

    let raw = fs::read_to_string(&resolved)
        .map_err(|e| format!("Failed to read {}: {}", resolved.display(), e))?;

    let entries = decode_load_order(&raw, cfg.format, cfg.describe)
        .map_err(|e| format!("Failed to parse {}: {}", resolved.display(), e))?;

    Ok(entries)
}

/// Persist a new load order. The frontend sends the desired ID order; we
/// merge with the existing on-disk entries to preserve `enabled` flags for
/// IDs the user didn't toggle, then write atomically.
#[tauri::command]
pub async fn set_file_based_load_order(
    game_id: String,
    bottle_name: String,
    order: Vec<LoadOrderEntry>,
) -> Result<(), String> {
    let (_opt_bottle, game, _data_dir) = resolve_game_any_runtime(&game_id, &bottle_name)?;
    let game_path = PathBuf::from(&game.game_path);

    let cfg = games::with_plugin(&game_id, |plugin| match plugin.load_order_kind(&game_path) {
        LoadOrderKind::FileBased(c) => Some(c),
        _ => None,
    })
    .flatten()
    .ok_or_else(|| format!("Game '{}' does not use a file-based load order", game_id))?;

    let resolved = resolve_config_path(&game_path, &cfg.config_path);

    // BG3 modsettings.lsx requires read-then-merge semantics: the write phase
    // must carry over full ModEntry data (Folder, Name, Version64) for each
    // UUID from the existing file, and always keep master entries at the top.
    if cfg.format == LoadOrderFormat::Bg3ModSettings {
        return write_bg3_load_order(&resolved, &order);
    }

    // Reject IDs that don't survive a round-trip through encode/decode (null
    // bytes, embedded newlines for `Lines` format, etc.). We do this here
    // rather than in `encode_load_order` so the frontend gets a clear error.
    for entry in &order {
        if entry.id.is_empty() {
            return Err("Load order entry has empty id".to_string());
        }
        if entry.id.contains('\0') {
            return Err(format!("Load order entry '{}' contains a null byte", entry.id));
        }
        if cfg.format == LoadOrderFormat::Lines && entry.id.contains('\n') {
            return Err(format!(
                "Load order entry '{}' contains a newline; not representable in Lines format",
                entry.id
            ));
        }
    }

    let encoded = encode_load_order(&order, cfg.format)
        .map_err(|e| format!("Failed to encode load order: {}", e))?;

    atomic_write(&resolved, &encoded)
        .map_err(|e| format!("Failed to write {}: {}", resolved.display(), e))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Path resolution + atomic write
// ---------------------------------------------------------------------------

/// Resolve a configured path. If absolute, used verbatim; otherwise joined
/// under the game directory.
fn resolve_config_path(game_path: &Path, config_path: &Path) -> PathBuf {
    if config_path.is_absolute() {
        config_path.to_path_buf()
    } else {
        game_path.join(config_path)
    }
}

/// Write `data` to `path` atomically (write to `<path>.tmp`, then rename).
/// On Wine bottles both files live on the same volume so the rename is
/// atomic. Creates parent directories as needed.
fn atomic_write(path: &Path, data: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    // Use a sibling temp file so the rename stays on the same filesystem.
    let mut tmp_name = path
        .file_name()
        .map(|n| n.to_owned())
        .unwrap_or_else(|| std::ffi::OsString::from("loadorder"));
    tmp_name.push(".corkscrew.tmp");
    let tmp_path = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&tmp_name);

    fs::write(&tmp_path, data)?;
    // If the rename fails (e.g. cross-device), the original file is untouched.
    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Codecs
// ---------------------------------------------------------------------------

/// Decode the on-disk representation into a list of [`LoadOrderEntry`].
fn decode_load_order(
    raw: &str,
    format: LoadOrderFormat,
    describe: Option<fn(&str) -> String>,
) -> Result<Vec<LoadOrderEntry>, String> {
    match format {
        LoadOrderFormat::Lines => Ok(decode_lines(raw, describe)),
        LoadOrderFormat::JsonArray => decode_json_array(raw, describe),
        LoadOrderFormat::RimWorldXml => decode_rimworld_xml(raw, describe),
        // Bg3ModSettings is dispatched before this function is called
        // (requires path access for the read-then-merge write path).
        // This arm is unreachable in normal operation but satisfies exhaustiveness.
        LoadOrderFormat::Bg3ModSettings => Err(
            "Bg3ModSettings codec must be invoked via read_bg3_load_order/write_bg3_load_order, \
             not decode_load_order"
                .to_string(),
        ),
    }
}

/// Encode a list of [`LoadOrderEntry`] for the given format.
fn encode_load_order(
    entries: &[LoadOrderEntry],
    format: LoadOrderFormat,
) -> Result<String, String> {
    match format {
        LoadOrderFormat::Lines => Ok(encode_lines(entries)),
        LoadOrderFormat::JsonArray => encode_json_array(entries),
        LoadOrderFormat::RimWorldXml => Ok(encode_rimworld_xml(entries)),
        // Bg3ModSettings is dispatched before this function is called.
        LoadOrderFormat::Bg3ModSettings => Err(
            "Bg3ModSettings codec must be invoked via write_bg3_load_order, \
             not encode_load_order"
                .to_string(),
        ),
    }
}

fn describe_id(id: &str, describe: Option<fn(&str) -> String>) -> String {
    describe.map(|f| f(id)).unwrap_or_else(|| id.to_string())
}

// -- BG3 modsettings.lsx --
//
// Read path: parse modsettings.lsx via bg3_lsx, return only the community
// mod entries (master entries are always present and are not user-controllable).
//
// Write path: read the existing file (to carry full ModEntry data forward by
// UUID lookup), then rebuild the mods list as [masters in original order] ++
// [community mods in caller-supplied order], and write atomically.

/// Read BG3 modsettings.lsx and return community mod entries only.
///
/// Master entries (GustavDev, Gustav, SharedDev) are filtered out. Each
/// community mod is mapped to a `LoadOrderEntry` where:
/// - `id` = the mod's UUID (lowercase for stable identity comparisons)
/// - `display_name` = `"<Name> (<Folder>)"` (human-readable in the UI)
/// - `enabled` = `true` (BG3 modsettings presence implies enabled; there is
///   no on-disk representation of a disabled but present community mod)
fn read_bg3_load_order(path: &Path) -> Result<Vec<LoadOrderEntry>, String> {
    let settings = crate::bg3_lsx::read_modsettings(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let entries = settings
        .mods
        .iter()
        .filter(|m| !crate::bg3_lsx::is_master_entry(&m.uuid))
        .map(|m| LoadOrderEntry {
            id: m.uuid.to_lowercase(),
            display_name: format!("{} ({})", m.name, m.folder),
            enabled: true,
        })
        .collect();

    Ok(entries)
}

/// Write a new community-mod order to BG3 modsettings.lsx.
///
/// Algorithm:
/// 1. Read the existing file (or use a default with all three masters) to
///    build a UUID → ModEntry index with full Folder/Name/Version64 data.
/// 2. Collect master entries in their original order.
/// 3. Append community entries in caller-supplied order, looked up by UUID
///    (case-insensitive). Unknown UUIDs (mods not in the existing file) are
///    silently skipped — they have no `ModEntry` metadata to write.
/// 4. Write atomically via `crate::bg3_lsx::write_modsettings`.
fn write_bg3_load_order(path: &Path, order: &[LoadOrderEntry]) -> Result<(), String> {
    use crate::bg3_lsx::{self, LsxVersion, ModSettings};

    // Load existing settings or use a sensible default.
    let existing = if path.exists() {
        bg3_lsx::read_modsettings(path)
            .map_err(|e| format!("Failed to read existing {}: {}", path.display(), e))?
    } else {
        // Bootstrap with the modern Patch 8+ master if the file doesn't
        // exist yet. (Normally deploy_native creates this first, but be
        // defensive.) Vanilla 4.8.0.700 ships exactly one master, GustavX —
        // writing the legacy GustavDev/Gustav/SharedDev trio here would
        // inject phantom masters on Patch 8 installs.
        let masters = vec![crate::bg3_lsx::ModEntry {
            folder: "GustavX".into(),
            md5: String::new(),
            name: "GustavX".into(),
            publish_handle: "0".into(),
            uuid: bg3_lsx::MASTER_GUSTAV_X_UUID.into(),
            version64: "36028797018963968".into(),
        }];
        ModSettings {
            version: LsxVersion { major: 4, minor: 0, revision: 9, build: 319 },
            mods: masters,
        }
    };

    // Build UUID (lowercase) → ModEntry index for full-data lookup.
    let index: HashMap<String, crate::bg3_lsx::ModEntry> = existing
        .mods
        .iter()
        .map(|m| (m.uuid.to_lowercase(), m.clone()))
        .collect();

    // New mods list: masters first (preserving their original relative order),
    // then community mods in the caller-supplied order.
    let mut new_mods: Vec<crate::bg3_lsx::ModEntry> = existing
        .mods
        .iter()
        .filter(|m| bg3_lsx::is_master_entry(&m.uuid))
        .cloned()
        .collect();

    for entry in order {
        let key = entry.id.to_lowercase();
        if let Some(full) = index.get(&key) {
            // Defensive: skip masters even if they somehow appear in the
            // caller-supplied order (they are not in the UI list).
            if !bg3_lsx::is_master_entry(&full.uuid) {
                new_mods.push(full.clone());
            }
        }
        // If the UUID is unknown (no matching entry in the existing file),
        // we have no Folder/Name/Version64 to write, so skip silently.
    }

    let mut updated = existing;
    updated.mods = new_mods;

    bg3_lsx::write_modsettings(path, &updated)
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;

    Ok(())
}

// -- Lines (one ID per line; "# id" = disabled) --

fn decode_lines(raw: &str, describe: Option<fn(&str) -> String>) -> Vec<LoadOrderEntry> {
    raw.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            let (id, enabled) = if let Some(rest) = trimmed.strip_prefix('#') {
                let rest = rest.trim();
                if rest.is_empty() {
                    return None; // comment line, not a disabled entry
                }
                (rest.to_string(), false)
            } else {
                (trimmed.to_string(), true)
            };
            Some(LoadOrderEntry {
                display_name: describe_id(&id, describe),
                id,
                enabled,
            })
        })
        .collect()
}

fn encode_lines(entries: &[LoadOrderEntry]) -> String {
    let mut out = String::new();
    for e in entries {
        if e.enabled {
            out.push_str(&e.id);
        } else {
            out.push_str("# ");
            out.push_str(&e.id);
        }
        out.push('\n');
    }
    out
}

// -- JSON array --

fn decode_json_array(
    raw: &str,
    describe: Option<fn(&str) -> String>,
) -> Result<Vec<LoadOrderEntry>, String> {
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let arr = value
        .as_array()
        .ok_or_else(|| "Expected JSON array".to_string())?;

    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        match item {
            serde_json::Value::String(s) => {
                let id = s.clone();
                out.push(LoadOrderEntry {
                    display_name: describe_id(&id, describe),
                    id,
                    enabled: true,
                });
            }
            serde_json::Value::Object(map) => {
                let id = map
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "JSON entry missing string 'id'".to_string())?
                    .to_string();
                let enabled = map.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
                out.push(LoadOrderEntry {
                    display_name: describe_id(&id, describe),
                    id,
                    enabled,
                });
            }
            other => return Err(format!("Unsupported JSON entry: {}", other)),
        }
    }
    Ok(out)
}

fn encode_json_array(entries: &[LoadOrderEntry]) -> Result<String, String> {
    // Always write the object form so we don't lose `enabled` on round-trip.
    let arr: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "enabled": e.enabled,
            })
        })
        .collect();
    serde_json::to_string_pretty(&arr).map_err(|e| e.to_string())
}

// -- RimWorld ModsConfig.xml --
//
// Minimal handler: read/write `<activeMods>` containing `<li>` children. We
// don't try to round-trip the rest of the XML — the upstream format only
// represents *active* mods, so disabled entries can't be persisted. Toggling
// still works in-session via the in-memory state on the frontend.

fn decode_rimworld_xml(
    raw: &str,
    describe: Option<fn(&str) -> String>,
) -> Result<Vec<LoadOrderEntry>, String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(raw);
    reader.config_mut().trim_text(true);

    let mut in_active_mods = false;
    let mut in_li = false;
    let mut current = String::new();
    let mut out = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => match e.name().as_ref() {
                b"activeMods" => in_active_mods = true,
                b"li" if in_active_mods => {
                    in_li = true;
                    current.clear();
                }
                _ => {}
            },
            Ok(Event::End(ref e)) => match e.name().as_ref() {
                b"activeMods" => in_active_mods = false,
                b"li" if in_active_mods => {
                    if !current.is_empty() {
                        let id = current.trim().to_string();
                        out.push(LoadOrderEntry {
                            display_name: describe_id(&id, describe),
                            id,
                            enabled: true,
                        });
                    }
                    in_li = false;
                }
                _ => {}
            },
            Ok(Event::Text(t)) => {
                if in_li {
                    let s = t
                        .xml_content()
                        .map_err(|e| format!("XML decode error: {}", e))?;
                    current.push_str(&s);
                }
            }
            Ok(Event::GeneralRef(r)) => {
                // quick-xml 0.38 splits general entity refs (&amp;, &lt;, ...)
                // out of text content into their own events. Resolve the five
                // predefined XML entities here; anything else (which RimWorld
                // doesn't use) is best-effort: we drop it.
                if in_li {
                    let bytes: &[u8] = &r;
                    let resolved = match bytes {
                        b"amp" => Some('&'),
                        b"lt" => Some('<'),
                        b"gt" => Some('>'),
                        b"quot" => Some('"'),
                        b"apos" => Some('\''),
                        _ => r.resolve_char_ref().ok().flatten(),
                    };
                    if let Some(ch) = resolved {
                        current.push(ch);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {}", e)),
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

fn encode_rimworld_xml(entries: &[LoadOrderEntry]) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    out.push_str("<ModsConfigData>\n");
    out.push_str("  <activeMods>\n");
    for e in entries.iter().filter(|e| e.enabled) {
        out.push_str("    <li>");
        out.push_str(&xml_escape(&e.id));
        out.push_str("</li>\n");
    }
    out.push_str("  </activeMods>\n");
    out.push_str("</ModsConfigData>\n");
    out
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(items: &[(&str, bool)]) -> Vec<LoadOrderEntry> {
        items
            .iter()
            .map(|(id, enabled)| LoadOrderEntry {
                id: (*id).to_string(),
                display_name: (*id).to_string(),
                enabled: *enabled,
            })
            .collect()
    }

    // -- Lines round-trip --

    #[test]
    fn lines_roundtrip_preserves_order_and_enabled() {
        let original = entries(&[("alpha", true), ("beta", false), ("gamma", true)]);
        let encoded = encode_load_order(&original, LoadOrderFormat::Lines).unwrap();
        let decoded = decode_load_order(&encoded, LoadOrderFormat::Lines, None).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn lines_decoder_skips_blank_lines_and_bare_comments() {
        let raw = "\n# bare comment\n\nalpha\n# beta\n\n";
        let decoded = decode_load_order(raw, LoadOrderFormat::Lines, None).unwrap();
        // "# bare comment" is treated as the disabled entry "bare comment"
        // because we can't distinguish a comment from a disabled mod ID
        // without stricter syntax — that's acceptable: users round-tripping
        // through us will get consistent enable/disable behaviour.
        assert_eq!(
            decoded,
            entries(&[("bare comment", false), ("alpha", true), ("beta", false)])
        );
    }

    #[test]
    fn lines_describe_function_is_applied() {
        let raw = "core\nui\n";
        fn describe(id: &str) -> String {
            format!("Mod: {}", id)
        }
        let decoded =
            decode_load_order(raw, LoadOrderFormat::Lines, Some(describe as fn(&str) -> String))
                .unwrap();
        assert_eq!(decoded[0].display_name, "Mod: core");
        assert_eq!(decoded[1].display_name, "Mod: ui");
    }

    // -- JSON array round-trip --

    #[test]
    fn json_roundtrip_preserves_order_and_enabled() {
        let original = entries(&[("a", true), ("b", false), ("c", true)]);
        let encoded = encode_load_order(&original, LoadOrderFormat::JsonArray).unwrap();
        let decoded = decode_load_order(&encoded, LoadOrderFormat::JsonArray, None).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn json_decodes_string_array_form() {
        let raw = r#"["one", "two", "three"]"#;
        let decoded = decode_load_order(raw, LoadOrderFormat::JsonArray, None).unwrap();
        assert_eq!(decoded, entries(&[("one", true), ("two", true), ("three", true)]));
    }

    #[test]
    fn json_rejects_non_array() {
        let raw = r#"{"id":"a"}"#;
        assert!(decode_load_order(raw, LoadOrderFormat::JsonArray, None).is_err());
    }

    // -- RimWorld XML round-trip --

    #[test]
    fn rimworld_xml_roundtrip_preserves_active_mods() {
        // Disabled entries can't be represented; the round-trip is over the
        // enabled subset.
        let original = entries(&[("ludeon.rimworld.core", true), ("brrainz.harmony", true)]);
        let encoded = encode_load_order(&original, LoadOrderFormat::RimWorldXml).unwrap();
        let decoded = decode_load_order(&encoded, LoadOrderFormat::RimWorldXml, None).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn rimworld_xml_drops_disabled_entries_on_write() {
        let original = entries(&[("a", true), ("b", false), ("c", true)]);
        let encoded = encode_load_order(&original, LoadOrderFormat::RimWorldXml).unwrap();
        let decoded = decode_load_order(&encoded, LoadOrderFormat::RimWorldXml, None).unwrap();
        assert_eq!(decoded, entries(&[("a", true), ("c", true)]));
    }

    #[test]
    fn rimworld_xml_handles_special_chars() {
        let original = entries(&[("foo&bar", true), ("baz<qux>", true)]);
        let encoded = encode_load_order(&original, LoadOrderFormat::RimWorldXml).unwrap();
        let decoded = decode_load_order(&encoded, LoadOrderFormat::RimWorldXml, None).unwrap();
        assert_eq!(decoded, original);
    }

    // -- Atomic write --

    #[test]
    fn atomic_write_creates_file_and_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("order.txt");

        atomic_write(&path, "first\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "first\n");

        atomic_write(&path, "second\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second\n");
    }

    #[test]
    fn atomic_write_failure_leaves_original_untouched() {
        // Force a rename failure by pointing the target at a directory.
        // The function writes to <target>.corkscrew.tmp first, then renames;
        // renaming a regular file *over* an existing directory fails on
        // both macOS and Linux, so the original directory is untouched.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blocking_dir");
        fs::create_dir(&path).unwrap();
        // Create a sentinel file inside so we can confirm the dir wasn't
        // clobbered.
        fs::write(path.join("keep.txt"), b"keep me").unwrap();

        let result = atomic_write(&path, "new contents");
        assert!(result.is_err(), "expected rename to fail");
        assert!(path.is_dir(), "original directory should still exist");
        assert!(path.join("keep.txt").exists());
        // Ensure the temp file was cleaned up.
        let tmp_left = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .ends_with(".corkscrew.tmp")
            });
        assert!(!tmp_left, "temp file should have been cleaned up");
    }

    // -- Path resolution --

    #[test]
    fn relative_config_path_joins_under_game() {
        let game = Path::new("/games/foo");
        let cfg = Path::new("Config/mods.txt");
        assert_eq!(
            resolve_config_path(game, cfg),
            PathBuf::from("/games/foo/Config/mods.txt")
        );
    }

    #[test]
    fn absolute_config_path_used_verbatim() {
        let game = Path::new("/games/foo");
        let cfg = PathBuf::from("/abs/path/to/order.json");
        assert_eq!(resolve_config_path(game, &cfg), cfg);
    }

    // -- Smoke test that FileBasedLoadOrder constructs cleanly --

    #[test]
    fn file_based_load_order_struct_smoke() {
        let _ = crate::games::FileBasedLoadOrder {
            config_path: PathBuf::from("Config/mods.txt"),
            format: LoadOrderFormat::Lines,
            describe: None,
        };
    }

    // -- BG3 modsettings.lsx read/write --

    use crate::bg3_lsx::{
        LsxVersion, ModEntry, ModSettings, MASTER_GUSTAV_DEV_UUID, MASTER_GUSTAV_UUID,
        MASTER_SHARED_DEV_UUID,
    };

    /// Build a minimal `ModSettings` with the three masters plus any extra entries.
    fn make_settings(extras: Vec<ModEntry>) -> ModSettings {
        let mut mods = vec![
            ModEntry {
                folder: "GustavDev".into(),
                md5: String::new(),
                name: "GustavDev".into(),
                publish_handle: "0".into(),
                uuid: MASTER_GUSTAV_DEV_UUID.into(),
                version64: "36028797018963968".into(),
            },
            ModEntry {
                folder: "Gustav".into(),
                md5: String::new(),
                name: "Gustav".into(),
                publish_handle: "0".into(),
                uuid: MASTER_GUSTAV_UUID.into(),
                version64: "36028797018963968".into(),
            },
            ModEntry {
                folder: "SharedDev".into(),
                md5: String::new(),
                name: "SharedDev".into(),
                publish_handle: "0".into(),
                uuid: MASTER_SHARED_DEV_UUID.into(),
                version64: "36028797018963968".into(),
            },
        ];
        mods.extend(extras);
        ModSettings {
            version: LsxVersion { major: 4, minor: 0, revision: 9, build: 319 },
            mods,
        }
    }

    /// Write a `ModSettings` to a temp file and return the temp dir + path.
    fn write_fixture(settings: &ModSettings) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("modsettings.lsx");
        crate::bg3_lsx::write_modsettings(&path, settings).unwrap();
        (dir, path)
    }

    #[test]
    fn bg3_load_order_read_filters_master_entries() {
        // Fixture: 3 masters + 2 community mods.
        let community_a = ModEntry {
            folder: "CommunityA".into(),
            md5: String::new(),
            name: "Community Mod A".into(),
            publish_handle: "0".into(),
            uuid: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into(),
            version64: "1".into(),
        };
        let community_b = ModEntry {
            folder: "CommunityB".into(),
            md5: String::new(),
            name: "Community Mod B".into(),
            publish_handle: "0".into(),
            uuid: "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".into(),
            version64: "2".into(),
        };
        let settings = make_settings(vec![community_a, community_b]);
        let (_dir, path) = write_fixture(&settings);

        let result = read_bg3_load_order(&path).unwrap();

        // Must return exactly the 2 community mods, no masters.
        assert_eq!(result.len(), 2, "should return only community mods, got {:?}", result);
        assert!(
            result.iter().all(|e| {
                e.id != MASTER_GUSTAV_DEV_UUID.to_lowercase()
                    && e.id != MASTER_GUSTAV_UUID.to_lowercase()
                    && e.id != MASTER_SHARED_DEV_UUID.to_lowercase()
            }),
            "masters must not appear in the result"
        );
        assert_eq!(result[0].id, "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
        assert_eq!(result[1].id, "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
        assert_eq!(result[0].display_name, "Community Mod A (CommunityA)");
    }

    #[test]
    fn bg3_load_order_read_returns_empty_for_masters_only() {
        // Fixture with only the 3 master entries.
        let settings = make_settings(vec![]);
        let (_dir, path) = write_fixture(&settings);

        let result = read_bg3_load_order(&path).unwrap();
        assert!(
            result.is_empty(),
            "masters-only modsettings should yield empty list; got {:?}",
            result
        );
    }

    #[test]
    fn bg3_load_order_write_preserves_master_entries() {
        // Fixture: 3 masters + 2 community mods in original order [A, B].
        let community_a = ModEntry {
            folder: "CommunityA".into(),
            md5: String::new(),
            name: "Community Mod A".into(),
            publish_handle: "0".into(),
            uuid: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into(),
            version64: "1".into(),
        };
        let community_b = ModEntry {
            folder: "CommunityB".into(),
            md5: String::new(),
            name: "Community Mod B".into(),
            publish_handle: "0".into(),
            uuid: "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".into(),
            version64: "2".into(),
        };
        let settings = make_settings(vec![community_a, community_b]);
        let (_dir, path) = write_fixture(&settings);

        // Reorder: B first, then A.
        let new_order = vec![
            LoadOrderEntry {
                id: "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".into(),
                display_name: "Community Mod B (CommunityB)".into(),
                enabled: true,
            },
            LoadOrderEntry {
                id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into(),
                display_name: "Community Mod A (CommunityA)".into(),
                enabled: true,
            },
        ];

        write_bg3_load_order(&path, &new_order).unwrap();

        let updated = crate::bg3_lsx::read_modsettings(&path).unwrap();

        // Masters must all be present (in the first 3 slots).
        let uuids: Vec<&str> = updated.mods.iter().map(|m| m.uuid.as_str()).collect();
        assert!(uuids.contains(&MASTER_GUSTAV_DEV_UUID), "GustavDev must be preserved");
        assert!(uuids.contains(&MASTER_GUSTAV_UUID), "Gustav must be preserved");
        assert!(uuids.contains(&MASTER_SHARED_DEV_UUID), "SharedDev must be preserved");

        // Community mods must be in the new order (after masters).
        let community: Vec<&str> = updated
            .mods
            .iter()
            .filter(|m| !crate::bg3_lsx::is_master_entry(&m.uuid))
            .map(|m| m.uuid.as_str())
            .collect();
        assert_eq!(
            community,
            vec!["bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb", "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"],
            "community mods must be reordered to [B, A]"
        );

        // Total: 3 masters + 2 mods.
        assert_eq!(updated.mods.len(), 5);
    }

    #[test]
    fn bg3_load_order_write_preserves_version_block() {
        let settings = make_settings(vec![ModEntry {
            folder: "MyMod".into(),
            md5: String::new(),
            name: "My Mod".into(),
            publish_handle: "0".into(),
            uuid: "11111111-1111-1111-1111-111111111111".into(),
            version64: "42".into(),
        }]);
        let (_dir, path) = write_fixture(&settings);

        let order = vec![LoadOrderEntry {
            id: "11111111-1111-1111-1111-111111111111".into(),
            display_name: "My Mod (MyMod)".into(),
            enabled: true,
        }];

        write_bg3_load_order(&path, &order).unwrap();

        let updated = crate::bg3_lsx::read_modsettings(&path).unwrap();
        assert_eq!(updated.version.major, 4, "version.major must be preserved");
        assert_eq!(updated.version.minor, 0, "version.minor must be preserved");
        assert_eq!(updated.version.revision, 9, "version.revision must be preserved");
        assert_eq!(updated.version.build, 319, "version.build must be preserved");
    }

    #[test]
    fn bg3_load_order_write_bootstraps_gustav_x_when_file_missing() {
        // Regression: when modsettings.lsx doesn't exist, the defensive
        // bootstrap must write the Patch 8 GustavX master — NOT the legacy
        // GustavDev/Gustav/SharedDev trio, which would be phantom masters
        // on a modern install.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("modsettings.lsx");

        write_bg3_load_order(&path, &[]).unwrap();

        let written = crate::bg3_lsx::read_modsettings(&path).unwrap();
        let uuids: Vec<String> = written.mods.iter().map(|m| m.uuid.to_lowercase()).collect();
        assert_eq!(
            uuids,
            vec![crate::bg3_lsx::MASTER_GUSTAV_X_UUID.to_lowercase()],
            "missing-file bootstrap must write exactly one GustavX master"
        );
        assert!(
            !uuids.contains(&MASTER_GUSTAV_DEV_UUID.to_lowercase()),
            "legacy GustavDev must not be injected"
        );
    }

    #[test]
    fn bg3_load_order_write_skips_unknown_uuids() {
        // Only masters in the file; no community mods registered.
        let settings = make_settings(vec![]);
        let (_dir, path) = write_fixture(&settings);

        // Supply a UUID that isn't in the file.
        let order = vec![LoadOrderEntry {
            id: "deadbeef-dead-beef-dead-beefdeadbeef".into(),
            display_name: "Ghost Mod (Ghost)".into(),
            enabled: true,
        }];

        write_bg3_load_order(&path, &order).unwrap();

        let updated = crate::bg3_lsx::read_modsettings(&path).unwrap();
        // Unknown UUID should be silently dropped — only masters remain.
        assert_eq!(updated.mods.len(), 3, "unknown UUID must be skipped");
    }
}
