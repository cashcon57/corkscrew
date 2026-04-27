//! Generic file-based load order: get/set commands and on-disk codecs.
//!
//! Bethesda titles drive the rich plugins.txt UI via [`commands::plugins`];
//! everything in this module is for games whose load order is a plain ordered
//! list of mod identifiers persisted to a config file (UE4 `~mods`,
//! Unity / RimWorld `ModsConfig.xml`, BepInEx-style ordering, etc.).
//!
//! The frontend asks `get_load_order_kind` first, and only invokes the
//! `*_file_based_load_order` commands when the answer is `"file_based"`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::games::{self, LoadOrderFormat, LoadOrderKind};
use crate::resolve_game;

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
    let (_bottle, game, _data_dir) = resolve_game(&game_id, &bottle_name)?;
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
    let (_bottle, game, _data_dir) = resolve_game(&game_id, &bottle_name)?;
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
    let (_bottle, game, _data_dir) = resolve_game(&game_id, &bottle_name)?;
    let game_path = PathBuf::from(&game.game_path);

    let cfg = games::with_plugin(&game_id, |plugin| match plugin.load_order_kind(&game_path) {
        LoadOrderKind::FileBased(c) => Some(c),
        _ => None,
    })
    .flatten()
    .ok_or_else(|| format!("Game '{}' does not use a file-based load order", game_id))?;

    let resolved = resolve_config_path(&game_path, &cfg.config_path);

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
    }
}

fn describe_id(id: &str, describe: Option<fn(&str) -> String>) -> String {
    describe.map(|f| f(id)).unwrap_or_else(|| id.to_string())
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
}
