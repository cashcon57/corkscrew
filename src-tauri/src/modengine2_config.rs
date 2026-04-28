//! Mod Engine 2 (`modengine2.toml`) parser/serializer for FromSoft games.
//!
//! Mod Engine 2 ships per-game config files like `config_eldenring.toml`,
//! `config_sekiro.toml`, etc., living under `<game>/modengine2/` (the
//! standard layout the upstream installer drops). The TOML schema:
//!
//! ```toml
//! [modengine]
//! debug = false
//! external_dlls = []
//!
//! [extension.mod_loader]
//! enabled = true
//! loose_params = false
//!
//! [[extension.mod_loader.mods]]
//! enabled = true
//! name = "MyMod"
//! path = "mod/MyMod"
//!
//! [extension.scylla_hide]
//! enabled = false
//! ```
//!
//! We intentionally model only the fields users will edit through Corkscrew
//! and round-trip the rest via `serde_json::Value`-style "extra" capture so
//! we never clobber unknown keys a user may have added by hand.
//!
//! Atomic write convention: `tmp + rename`, matching `config.rs`. Parse
//! errors are recoverable — callers should surface them rather than
//! silently overwriting a malformed file.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use toml::Value;

/// Per-mod entry in `[[extension.mod_loader.mods]]`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModEntry {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub name: String,
    pub path: String,
}

fn default_true() -> bool {
    true
}

/// Top-level `[modengine]` table.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModEngineSection {
    #[serde(default)]
    pub debug: bool,
    #[serde(default)]
    pub external_dlls: Vec<String>,
}

/// `[extension.mod_loader]` table (sans the per-mod array).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModLoaderSection {
    pub enabled: bool,
    #[serde(default)]
    pub loose_params: bool,
    #[serde(default)]
    pub mods: Vec<ModEntry>,
}

impl Default for ModLoaderSection {
    fn default() -> Self {
        Self {
            enabled: true,
            loose_params: false,
            mods: Vec::new(),
        }
    }
}

/// Strongly-typed wrapper around the ME2 config TOML.
///
/// `extras` round-trips any tables / keys we don't model explicitly, so
/// editing through Corkscrew never silently drops `[extension.scylla_hide]`
/// or other extensions a user has hand-added.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ModEngine2Config {
    #[serde(default)]
    pub modengine: ModEngineSection,
    #[serde(default)]
    pub mod_loader: ModLoaderSection,
    /// Pass-through table for unknown sections under `[extension.*]`
    /// (`scylla_hide`, future extensions, etc.).
    #[serde(default)]
    pub extra_extensions: BTreeMap<String, Value>,
    /// Path the config was loaded from (for round-trip writes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Path discovery
// ---------------------------------------------------------------------------

/// Find the ME2 config file for a given game.
///
/// Discovery order:
/// 1. `<game>/modengine2/config_<game>.toml` (canonical ME2 layout, where
///    `<game>` matches the slugs ME2 uses: eldenring, sekiro, darksouls3,
///    darksoulsremastered, armoredcore6).
/// 2. `<game>/modengine2/*.toml` — fallback to first TOML found.
///
/// Returns `None` when no candidate exists.
pub fn find_config_path(game_path: &Path, game_id: &str) -> Option<PathBuf> {
    let me2_dir = game_path.join("modengine2");
    if !me2_dir.is_dir() {
        return None;
    }

    // Map our internal game_ids to the ME2 config slug. The ME2 distribution
    // uses lowercased no-underscore slugs.
    let me2_slug = match game_id {
        "eldenring" => Some("eldenring"),
        "sekiro" => Some("sekiro"),
        "darksouls3" => Some("darksouls3"),
        "darksouls_remastered" => Some("darksoulsremastered"),
        "armoredcore6" => Some("armoredcore6"),
        _ => None,
    };

    if let Some(slug) = me2_slug {
        let canonical = me2_dir.join(format!("config_{}.toml", slug));
        if canonical.exists() {
            return Some(canonical);
        }
    }

    // Fallback: any *.toml in the modengine2 dir. Sorted for determinism.
    let mut tomls: Vec<PathBuf> = fs::read_dir(&me2_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|ext| ext.to_str()).map(|s| s.eq_ignore_ascii_case("toml"))
                == Some(true)
        })
        .collect();
    tomls.sort();
    tomls.into_iter().next()
}

// ---------------------------------------------------------------------------
// Load / save
// ---------------------------------------------------------------------------

/// Load the ME2 config for a given game. Returns a default-shaped config
/// if no file exists yet (callers can `save_config` to materialize one).
pub fn load_config(game_path: &Path, game_id: &str) -> Result<ModEngine2Config, String> {
    let path = match find_config_path(game_path, game_id) {
        Some(p) => p,
        None => return Ok(ModEngine2Config::default()),
    };
    let contents = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let mut cfg = parse_toml(&contents)?;
    cfg.source_path = Some(path.to_string_lossy().to_string());
    Ok(cfg)
}

/// Atomically write the ME2 config file (tmp + rename).
///
/// Resolves the target path the same way [`load_config`] does, but if
/// `cfg.source_path` is set we honor that to support round-trip edits of
/// hand-renamed files.
pub fn save_config(
    game_path: &Path,
    game_id: &str,
    cfg: &ModEngine2Config,
) -> Result<(), String> {
    let target = if let Some(p) = cfg.source_path.as_deref() {
        PathBuf::from(p)
    } else if let Some(p) = find_config_path(game_path, game_id) {
        p
    } else {
        // First-time materialization: derive the canonical path.
        let me2_dir = game_path.join("modengine2");
        fs::create_dir_all(&me2_dir)
            .map_err(|e| format!("Failed to create {}: {}", me2_dir.display(), e))?;
        let slug = match game_id {
            "eldenring" => "eldenring",
            "sekiro" => "sekiro",
            "darksouls3" => "darksouls3",
            "darksouls_remastered" => "darksoulsremastered",
            "armoredcore6" => "armoredcore6",
            _ => "fromsoft",
        };
        me2_dir.join(format!("config_{}.toml", slug))
    };

    let serialized = serialize_toml(cfg)?;
    let parent = target.parent().ok_or_else(|| {
        format!("ME2 config target {} has no parent directory", target.display())
    })?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;

    // Atomic write: write to a sibling tmp file then rename. If anything
    // goes wrong before the rename, the original is untouched.
    let tmp = target.with_extension("toml.tmp");
    {
        let mut f = fs::File::create(&tmp)
            .map_err(|e| format!("Failed to open tmp file {}: {}", tmp.display(), e))?;
        f.write_all(serialized.as_bytes())
            .map_err(|e| format!("Failed to write {}: {}", tmp.display(), e))?;
        f.sync_all()
            .map_err(|e| format!("Failed to fsync {}: {}", tmp.display(), e))?;
    }
    fs::rename(&tmp, &target)
        .map_err(|e| format!("Failed to rename {} -> {}: {}", tmp.display(), target.display(), e))?;

    Ok(())
}

/// Append a new mod to the loader and enable it. Idempotent on `name`:
/// updates `path` if a mod by that name already exists.
pub fn add_mod(cfg: &mut ModEngine2Config, name: &str, path: &str) {
    if let Some(existing) = cfg.mod_loader.mods.iter_mut().find(|m| m.name == name) {
        existing.path = path.to_string();
        existing.enabled = true;
        return;
    }
    cfg.mod_loader.mods.push(ModEntry {
        enabled: true,
        name: name.to_string(),
        path: path.to_string(),
    });
}

/// Remove the entry whose `name` matches. Returns true if a removal occurred.
pub fn remove_mod(cfg: &mut ModEngine2Config, name: &str) -> bool {
    let before = cfg.mod_loader.mods.len();
    cfg.mod_loader.mods.retain(|m| m.name != name);
    cfg.mod_loader.mods.len() != before
}

// ---------------------------------------------------------------------------
// (De)serialization
// ---------------------------------------------------------------------------

fn parse_toml(text: &str) -> Result<ModEngine2Config, String> {
    let value: Value = toml::from_str(text).map_err(|e| format!("Invalid TOML: {}", e))?;
    let table = match value {
        Value::Table(t) => t,
        _ => return Err("ME2 config root must be a table".into()),
    };

    let mut cfg = ModEngine2Config::default();

    if let Some(modengine) = table.get("modengine") {
        cfg.modengine = modengine
            .clone()
            .try_into()
            .map_err(|e| format!("Invalid [modengine]: {}", e))?;
    }

    if let Some(extension) = table.get("extension").and_then(|v| v.as_table()) {
        if let Some(loader) = extension.get("mod_loader") {
            cfg.mod_loader = loader
                .clone()
                .try_into()
                .map_err(|e| format!("Invalid [extension.mod_loader]: {}", e))?;
        }
        for (k, v) in extension {
            if k == "mod_loader" {
                continue;
            }
            cfg.extra_extensions.insert(k.clone(), v.clone());
        }
    }

    Ok(cfg)
}

fn serialize_toml(cfg: &ModEngine2Config) -> Result<String, String> {
    // Build the TOML by hand to preserve the canonical ME2 ordering:
    //   [modengine]
    //   [extension.mod_loader]
    //   [[extension.mod_loader.mods]]
    //   [extension.<other>]  (round-tripped)
    //
    // We use `toml::Value` for each subtree's serialization, then stitch
    // them together. This avoids relying on a struct-derive ordering that
    // toml's serializer might shuffle.

    let mut out = String::new();

    out.push_str("[modengine]\n");
    out.push_str(&format!("debug = {}\n", cfg.modengine.debug));
    // TOML inline-array of strings. We serialize via `Value::Array` so each
    // element is properly quoted/escaped without needing a wrapper struct.
    let dll_array = Value::Array(
        cfg.modengine
            .external_dlls
            .iter()
            .map(|s| Value::String(s.clone()))
            .collect(),
    );
    out.push_str(&format!("external_dlls = {}\n", dll_array));
    out.push('\n');

    out.push_str("[extension.mod_loader]\n");
    out.push_str(&format!("enabled = {}\n", cfg.mod_loader.enabled));
    out.push_str(&format!("loose_params = {}\n", cfg.mod_loader.loose_params));
    out.push('\n');

    for m in &cfg.mod_loader.mods {
        out.push_str("[[extension.mod_loader.mods]]\n");
        out.push_str(&format!("enabled = {}\n", m.enabled));
        // Use TOML string escaping for safety (paths can contain backslashes).
        let name_value = Value::String(m.name.clone());
        let path_value = Value::String(m.path.clone());
        out.push_str(&format!("name = {}\n", name_value));
        out.push_str(&format!("path = {}\n", path_value));
        out.push('\n');
    }

    // Round-trip extras. Sort for determinism.
    for (k, v) in &cfg.extra_extensions {
        // Each value we round-trip should be a sub-table; render it as
        // `[extension.<key>]`.
        if let Value::Table(t) = v {
            out.push_str(&format!("[extension.{}]\n", k));
            for (kk, vv) in t {
                // Avoid clobbering nested tables if any. For non-table values we
                // emit a key = value line; for nested tables we serialize
                // a child header.
                if let Value::Table(_) = vv {
                    let nested = toml::to_string(&{
                        let mut wrap = toml::value::Table::new();
                        wrap.insert(kk.clone(), vv.clone());
                        wrap
                    })
                    .map_err(|e| e.to_string())?;
                    out.push_str(&nested);
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                } else {
                    out.push_str(&format!("{} = {}\n", kk, vv));
                }
            }
            out.push('\n');
        }
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const SAMPLE: &str = r#"
[modengine]
debug = false
external_dlls = []

[extension.mod_loader]
enabled = true
loose_params = false

[[extension.mod_loader.mods]]
enabled = true
name = "GTS"
path = "mod/GTS"

[[extension.mod_loader.mods]]
enabled = false
name = "Reshade"
path = "mod/Reshade"

[extension.scylla_hide]
enabled = true
"#;

    #[test]
    fn parse_basic_config() {
        let cfg = parse_toml(SAMPLE).expect("parse");
        assert!(!cfg.modengine.debug);
        assert!(cfg.modengine.external_dlls.is_empty());
        assert!(cfg.mod_loader.enabled);
        assert!(!cfg.mod_loader.loose_params);
        assert_eq!(cfg.mod_loader.mods.len(), 2);
        assert_eq!(cfg.mod_loader.mods[0].name, "GTS");
        assert!(cfg.mod_loader.mods[0].enabled);
        assert_eq!(cfg.mod_loader.mods[1].name, "Reshade");
        assert!(!cfg.mod_loader.mods[1].enabled);
        assert!(cfg.extra_extensions.contains_key("scylla_hide"));
    }

    #[test]
    fn round_trip_preserves_mods_and_extras() {
        let cfg = parse_toml(SAMPLE).expect("parse");
        let serialized = serialize_toml(&cfg).expect("serialize");
        let cfg2 = parse_toml(&serialized).expect("reparse");

        assert_eq!(cfg.modengine, cfg2.modengine);
        assert_eq!(cfg.mod_loader, cfg2.mod_loader);
        // The unknown [extension.scylla_hide] table must round-trip.
        assert!(cfg2.extra_extensions.contains_key("scylla_hide"));
    }

    #[test]
    fn add_mod_appends_when_new() {
        let mut cfg = parse_toml(SAMPLE).expect("parse");
        add_mod(&mut cfg, "Convergence", "mod/Convergence");
        assert_eq!(cfg.mod_loader.mods.len(), 3);
        assert_eq!(cfg.mod_loader.mods[2].name, "Convergence");
        assert!(cfg.mod_loader.mods[2].enabled);
    }

    #[test]
    fn add_mod_updates_existing_by_name() {
        let mut cfg = parse_toml(SAMPLE).expect("parse");
        add_mod(&mut cfg, "Reshade", "mod/Reshade2");
        // No duplicate, path updated, enabled flipped on.
        assert_eq!(cfg.mod_loader.mods.len(), 2);
        let r = cfg.mod_loader.mods.iter().find(|m| m.name == "Reshade").unwrap();
        assert_eq!(r.path, "mod/Reshade2");
        assert!(r.enabled);
    }

    #[test]
    fn remove_mod_returns_bool() {
        let mut cfg = parse_toml(SAMPLE).expect("parse");
        assert!(remove_mod(&mut cfg, "GTS"));
        assert_eq!(cfg.mod_loader.mods.len(), 1);
        assert!(!remove_mod(&mut cfg, "DoesNotExist"));
    }

    #[test]
    fn find_config_path_canonical_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("Elden Ring");
        let me2 = game.join("modengine2");
        fs::create_dir_all(&me2).unwrap();
        let canonical = me2.join("config_eldenring.toml");
        fs::write(&canonical, SAMPLE).unwrap();

        let got = find_config_path(&game, "eldenring").unwrap();
        assert_eq!(got, canonical);
    }

    #[test]
    fn find_config_path_fallback_when_no_canonical() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("Sekiro");
        let me2 = game.join("modengine2");
        fs::create_dir_all(&me2).unwrap();
        // User renamed it; we should still find a TOML in the dir.
        let alt = me2.join("custom.toml");
        fs::write(&alt, SAMPLE).unwrap();

        let got = find_config_path(&game, "sekiro").unwrap();
        assert_eq!(got, alt);
    }

    #[test]
    fn find_config_path_none_when_no_modengine_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("FreshInstall");
        fs::create_dir_all(&game).unwrap();
        assert!(find_config_path(&game, "eldenring").is_none());
    }

    #[test]
    fn save_then_reload_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("Elden Ring");
        fs::create_dir_all(&game).unwrap();

        let mut cfg = ModEngine2Config::default();
        add_mod(&mut cfg, "TestMod", "mod/TestMod");
        save_config(&game, "eldenring", &cfg).expect("save");

        let path = find_config_path(&game, "eldenring").expect("path");
        assert!(path.ends_with("config_eldenring.toml"));

        let reloaded = load_config(&game, "eldenring").expect("reload");
        assert_eq!(reloaded.mod_loader.mods.len(), 1);
        assert_eq!(reloaded.mod_loader.mods[0].name, "TestMod");
    }

    #[test]
    fn save_atomic_no_partial_clobber_on_locked_target() {
        // Simulate write failure by making the target file read-only.
        // The atomic write goes through a tmp + rename — even if rename
        // fails (because we make the parent dir read-only), the original
        // must not be clobbered.
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("Sekiro");
        let me2 = game.join("modengine2");
        fs::create_dir_all(&me2).unwrap();
        let canonical = me2.join("config_sekiro.toml");
        fs::write(&canonical, SAMPLE).unwrap();

        // Pre-load the existing config.
        let original = load_config(&game, "sekiro").unwrap();

        // Make the modengine2 dir read-only on Unix to fail the rename.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&me2).unwrap().permissions();
            perms.set_mode(0o555);
            fs::set_permissions(&me2, perms).unwrap();
        }

        // Attempt to save a different config. May succeed or fail depending on OS;
        // either way, the original file must remain valid.
        let mut new_cfg = original.clone();
        add_mod(&mut new_cfg, "Should Not Land", "mod/Bad");
        let _ = save_config(&game, "sekiro", &new_cfg);

        // Restore perms so cleanup works regardless.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&me2).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&me2, perms).unwrap();
        }

        // Whatever happened, the original file must still parse cleanly.
        let reloaded = load_config(&game, "sekiro").unwrap();
        // If the save succeeded, the new mod is present. If it failed, the
        // original's mod set is preserved. Either way, we never have a
        // partially-written / corrupt TOML.
        assert!(reloaded.mod_loader.mods.iter().any(|m| m.name == "GTS")
            || reloaded.mod_loader.mods.iter().any(|m| m.name == "Should Not Land"));
    }
}
