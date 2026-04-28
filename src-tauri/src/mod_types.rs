//! Mod-type registry — heuristics for routing mods to the correct install
//! directory based on archive contents.
//!
//! ## Why
//!
//! Bethesda games (Skyrim, Fallout) have a single, well-known data directory
//! (`<game>/Data`) into which every mod merges. Their [`GamePlugin`]
//! implementations return that directory from `get_data_dir()`, and the
//! installer copies extracted files in directly.
//!
//! Other games are more varied:
//!
//! - **BepInEx-based** games (Valheim, RoR2, Subnautica, etc.) want plugins
//!   in `BepInEx/plugins/<modname>/`.
//! - **Stardew Valley / SMAPI** wants `<game>/Mods/<modname>/`.
//! - **Unreal Engine** games want `.pak` files in
//!   `<game>/<project>/Content/Paks/~mods/`.
//! - **UE4SS** Lua mods want `<inferred>/Binaries/Win64/Mods/<modname>/`.
//! - **RimWorld** wants `<game>/Mods/<modname>/`.
//!
//! For unknown games (where no [`GamePlugin`] matches), we previously fell
//! back to `data_dir = game_path` and dumped everything in the game root.
//! That works for Bethesda-shaped archives by coincidence but routes
//! BepInEx / SMAPI / Unreal mods to the wrong place.
//!
//! ## Design — Vortex parity
//!
//! Vortex registers mod types via
//! `registerModType(id, priority, isSupported, getPath)`. Each type
//! self-declares which archives it claims and the install path. The highest
//! priority `isSupported` match wins.
//!
//! [`detect_mod_type`] walks the registered types in priority order and
//! returns the first match. [`resolve_install_target`] additionally computes
//! the absolute install path, including a per-mod subfolder when appropriate
//! (BepInEx plugins want their own dir; UE pak files merge into `~mods/`).

use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// ModType
// ---------------------------------------------------------------------------

/// A mod-type heuristic. See module docs.
pub struct ModType {
    /// Stable identifier used in DB / logs. e.g. `"BepInEx"`, `"SMAPI"`.
    pub id: &'static str,
    /// Human-readable name shown in UI / logs.
    pub display_name: &'static str,
    /// Higher priority wins when multiple types match. Range [0, 100].
    pub priority: u8,
    /// Predicate run against the archive's relative entry list. Return true
    /// if this mod type claims the archive.
    pub detect: fn(&[String]) -> bool,
    /// Compute the install path relative to (or rooted at) `game_path` for
    /// the given `mod_name`. The function may ignore `mod_name` if files
    /// merge into a shared parent (UE paks, BepInExPack bootstrap).
    pub install_path: fn(game_path: &Path, mod_name: &str) -> PathBuf,
    /// If true, the install path includes a per-mod subfolder (e.g.
    /// `BepInEx/plugins/MyMod/`); each mod gets its own directory.
    /// If false, files merge into a shared parent (e.g. UE pak `~mods/`).
    pub per_mod_subfolder: bool,
}

/// Result of [`resolve_install_target`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallTarget {
    /// Absolute deploy directory. When `per_mod_subfolder` is true, this
    /// already includes the per-mod subfolder.
    pub target_dir: PathBuf,
    /// Whether `target_dir` is a per-mod subfolder. Callers may use this to
    /// decide if existing files at `target_dir` represent a stale install
    /// of *this* mod (and can be wiped) or a shared deploy directory (and
    /// must not be touched).
    pub per_mod_subfolder: bool,
    /// Identifier of the matched [`ModType`] (or `"Generic_Subfolder"` for
    /// the fallback).
    pub type_id: &'static str,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Sanitize a string for use as a directory name. Removes characters that
/// are invalid on Windows (the most restrictive of the supported targets)
/// and trims surrounding whitespace and dots. Empty results fall back to
/// `"mod"` so we never create a `""` directory.
fn sanitize_dir_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        match ch {
            // Forbidden on NTFS / common archive consumers.
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => out.push('_'),
            c if (c as u32) < 0x20 => out.push('_'),
            c => out.push(c),
        }
    }
    let trimmed = out.trim_matches(|c: char| c.is_whitespace() || c == '.');
    if trimmed.is_empty() {
        "mod".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Lower-case helper: returns true iff any entry path's component (split on
/// `/` or `\`) equals one of `targets` (case-insensitive). Cheap for the
/// detect-fn use cases; we don't bother with `Path` parsing to avoid the
/// cross-platform dance for `\` separators in archive paths.
fn any_entry_starts_with(entries: &[String], prefix: &str) -> bool {
    let prefix_lower = prefix.to_lowercase();
    entries
        .iter()
        .any(|e| e.replace('\\', "/").to_lowercase().starts_with(&prefix_lower))
}

fn any_entry_ext_eq(entries: &[String], ext: &str) -> bool {
    let ext_lower = ext.to_lowercase();
    entries.iter().any(|e| {
        Path::new(e)
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| x.eq_ignore_ascii_case(&ext_lower))
            .unwrap_or(false)
    })
}

fn any_entry_filename_eq(entries: &[String], filename: &str) -> bool {
    let filename_lower = filename.to_lowercase();
    entries.iter().any(|e| {
        Path::new(e)
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.eq_ignore_ascii_case(&filename_lower))
            .unwrap_or(false)
    })
}

/// Returns true if any `entries[i]` ends (case-insensitive) in `suffix`.
fn any_entry_ends_with(entries: &[String], suffix: &str) -> bool {
    let suffix_lower = suffix.to_lowercase();
    entries
        .iter()
        .any(|e| e.replace('\\', "/").to_lowercase().ends_with(&suffix_lower))
}

/// Depth of an entry path = number of `/`-separated components, *not*
/// counting the final filename. `"manifest.json"` has depth 0;
/// `"sub/manifest.json"` has depth 1.
fn entry_depth(path: &str) -> usize {
    path.replace('\\', "/").trim_matches('/').split('/').count().saturating_sub(1)
}

// ---------------------------------------------------------------------------
// Detect functions
// ---------------------------------------------------------------------------

/// BepInExPack bootstrap — the pack ships `BepInExPack/winhttp.dll` /
/// `BepInExPack/BepInEx/`. Distinct from a regular BepInEx plugin.
fn detect_bepinex_pack(entries: &[String]) -> bool {
    any_entry_starts_with(entries, "BepInExPack/winhttp.dll")
        || any_entry_starts_with(entries, "BepInExPack/BepInEx/")
}

/// Regular BepInEx plugin — payload is a DLL under `BepInEx/plugins/` or
/// just `plugins/` (some authors strip the BepInEx prefix).
fn detect_bepinex_plugin(entries: &[String]) -> bool {
    let has_dll = any_entry_ext_eq(entries, "dll");
    let in_bepinex_plugins = any_entry_starts_with(entries, "BepInEx/plugins/");
    let in_loose_plugins = any_entry_starts_with(entries, "plugins/");
    has_dll && (in_bepinex_plugins || in_loose_plugins)
}

/// SMAPI (Stardew Modding API) — every SMAPI mod ships a `manifest.json`
/// alongside an assembly DLL. The manifest is at depth 0 (loose) or 1
/// (one wrapper dir).
fn detect_smapi(entries: &[String]) -> bool {
    let manifest = entries.iter().any(|e| {
        let p = e.replace('\\', "/");
        Path::new(&p)
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.eq_ignore_ascii_case("manifest.json"))
            .unwrap_or(false)
            && entry_depth(&p) <= 2
    });
    manifest && any_entry_ext_eq(entries, "dll")
}

/// Unreal Engine pak files at top level or under a `Paks/` directory.
fn detect_ue_paks(entries: &[String]) -> bool {
    if !any_entry_ext_eq(entries, "pak") {
        return false;
    }
    // Either top-level paks or under a Paks/ subdir; we don't insist on
    // either, just that paks exist somewhere — the install path falls back
    // to `<game>/Paks/~mods/` for the latter and the project-relative
    // path for the former.
    true
}

/// UE4SS Lua mods or the UE4SS bootstrap itself.
///
/// UE4SS Lua mods place files at `Mods/<name>/Scripts/main.lua` or
/// `Mods/<name>/main.lua`. The UE4SS bootstrap itself ships `dwmapi.dll`
/// alongside `UE4SS-settings.ini`.
fn detect_ue4ss(entries: &[String]) -> bool {
    let lua_mod = entries.iter().any(|e| {
        let p = e.replace('\\', "/").to_lowercase();
        (p.contains("/scripts/main.lua") && p.starts_with("mods/"))
            || (p.starts_with("mods/") && p.ends_with("/main.lua"))
    });
    let bootstrap = any_entry_filename_eq(entries, "dwmapi.dll")
        && any_entry_filename_eq(entries, "UE4SS-settings.ini");
    lua_mod || bootstrap
}

/// RimWorld mod — every RimWorld mod has `About/About.xml`.
fn detect_rimworld(entries: &[String]) -> bool {
    any_entry_ends_with(entries, "/About/About.xml")
        || entries.iter().any(|e| {
            e.replace('\\', "/").eq_ignore_ascii_case("About/About.xml")
        })
}

/// The Sims 4 `.package` archive (custom content / gameplay tuning).
///
/// Detection is generic by file extension — `.package` is essentially
/// Sims-4-specific (the only other widespread `.package` consumer is
/// The Sims 3, which Corkscrew doesn't support). Per-mod subfolder is
/// enabled so file conflicts between authors stay localized.
fn detect_sims4_package(entries: &[String]) -> bool {
    any_entry_ext_eq(entries, "package")
}

/// The Sims 4 `.ts4script` Python mod. Loaded by exact filename from
/// the `Mods/` root, so per-mod subfolder is **disabled** — the script
/// must live directly under `Mods/` (or one level deep, but flat keeps
/// load behaviour predictable).
fn detect_sims4_script(entries: &[String]) -> bool {
    any_entry_ext_eq(entries, "ts4script")
}

// --- GTA V detect functions ---
//
// NOTE on namespace overlap: the GTA V mod types fire purely on file shape
// (`.asi`, `dlcpacks/`, `scripts/*.dll`). They are NOT gated by `game_id` —
// the registry has no notion of game context. This is intentional and
// matches the existing Sims4_Package / SMAPI / BepInEx pattern.
//
// In practice the overlap is not a problem because:
// - Bethesda titles set `use_legacy_data_dir() == true` and bypass the
//   mod-types registry entirely (every mod merges into `<game>/Data`).
// - The remaining games whose archives could shadow GTA V shapes (e.g.
//   a Crimson Desert mod that ships a `dinput8.dll`) match higher-priority
//   types like `Generic_BepInExPack_Bootstrap` (95) before reaching these.
// - GTA V's own archives (.asi, dlcpacks/) have no other mainstream consumer.

/// Detect a GTA V dlcpack add-on. Two shapes:
///   1. Archive root contains a `dlcpacks/<pack>/dlc.rpf` tree.
///   2. Archive root IS a single dlcpack folder containing `dlc.rpf`
///      (e.g. `MyDLC/dlc.rpf` with no leading `dlcpacks/`).
fn detect_gtav_dlcpack(entries: &[String]) -> bool {
    let normalised: Vec<String> = entries
        .iter()
        .map(|e| e.replace('\\', "/").to_lowercase())
        .collect();
    let has_dlcpacks_tree = normalised
        .iter()
        .any(|e| e.starts_with("dlcpacks/") && e.ends_with("/dlc.rpf"));
    if has_dlcpacks_tree {
        return true;
    }
    // Bare `<pack>/dlc.rpf` shape — exactly one dir level then `dlc.rpf`.
    normalised.iter().any(|e| {
        let parts: Vec<&str> = e.split('/').collect();
        parts.len() == 2 && parts[1] == "dlc.rpf" && !parts[0].is_empty()
    })
}

/// Detect a GTA V ASI / Lua-script mod (Alexander Blade ASI loader plus
/// Headscript LUA Plugin payloads).
fn detect_gtav_asi_script(entries: &[String]) -> bool {
    if any_entry_ext_eq(entries, "asi") {
        return true;
    }
    // Lua Plugin scripts live under `scripts/addins/`.
    entries.iter().any(|e| {
        let p = e.replace('\\', "/").to_lowercase();
        p.starts_with("scripts/addins/") && p.ends_with(".lua")
    })
}

/// Detect a GTA V .NET (SHVDN) script mod — `.dll` files referencing the
/// SHVDN runtime, OR DLLs nested under a top-level `scripts/` dir.
fn detect_gtav_net_script(entries: &[String]) -> bool {
    let normalised: Vec<String> = entries
        .iter()
        .map(|e| e.replace('\\', "/").to_lowercase())
        .collect();
    let has_shvdn_ref = normalised
        .iter()
        .any(|e| e.contains("scripthookvdotnet") && e.ends_with(".dll"));
    let in_scripts = normalised
        .iter()
        .any(|e| e.starts_with("scripts/") && e.ends_with(".dll"));
    has_shvdn_ref || in_scripts
}

/// 3DMigoto-family mod (GIMI / SRMI / ZZMI / Honkai-3rd HIMI). The shared
/// signature is a 3DMigoto `.ini` config file alongside binary buffer / index
/// files (`.buf`, `.ib`) — the latter are unique to 3DMigoto-style asset
/// swaps and not produced by any other modding tool, so their presence is
/// the cleanest disambiguator from generic INI archives.
fn detect_gimi(entries: &[String]) -> bool {
    let has_ini = any_entry_ext_eq(entries, "ini");
    let has_buffer = any_entry_ext_eq(entries, "buf") || any_entry_ext_eq(entries, "ib");
    has_ini && has_buffer
}

/// Mod Engine 2 (FromSoft games — Sekiro, Elden Ring, DS3, DS:R, AC6).
/// Detects archives shipping FromSoft asset directories OR a regulation.bin.
///
/// Mod Engine 2 mods install per-mod under `<game>/mod/<modname>/`. Most
/// mods ship just `regulation.bin` (single-file) or one of the FromSoft
/// asset dirs (`parts/`, `event/`, `chr/`, `msg/`, `param/`, etc.). Some
/// also include a `modengine2.toml` config the loader reads.
fn detect_modengine2(entries: &[String]) -> bool {
    let normalised: Vec<String> = entries
        .iter()
        .map(|e| e.replace('\\', "/").to_lowercase())
        .collect();
    // regulation.bin at any depth (often shipped solo)
    let has_regulation = normalised.iter().any(|e| e.ends_with("regulation.bin"));
    // FromSoft asset dirs at depth ≤ 2 (top-level or one wrapper folder)
    let has_fs_asset_dir = normalised.iter().any(|e| {
        let parts: Vec<&str> = e.split('/').collect();
        if parts.len() < 2 || parts.len() > 4 {
            return false;
        }
        let segment = if parts.len() <= 2 {
            parts[0]
        } else {
            parts[parts.len() - 2]
        };
        matches!(
            segment,
            "parts" | "event" | "chr" | "msg" | "param" | "menu" | "facegen" | "sfx"
        )
    });
    let has_me2_toml = normalised.iter().any(|e| e.ends_with("modengine2.toml"));
    has_regulation || has_fs_asset_dir || has_me2_toml
}

/// Generic fallback — always matches. Lowest priority.
fn detect_generic(_entries: &[String]) -> bool {
    true
}

// ---------------------------------------------------------------------------
// Install-path functions
// ---------------------------------------------------------------------------

/// BepInExPack bootstrap extracts directly to the game root — the pack
/// ships its own folder structure (`BepInExPack/...`) which lays out
/// alongside the game exe.
fn path_bepinex_pack(game_path: &Path, _mod_name: &str) -> PathBuf {
    game_path.to_path_buf()
}

fn path_bepinex_plugin(game_path: &Path, mod_name: &str) -> PathBuf {
    game_path
        .join("BepInEx")
        .join("plugins")
        .join(sanitize_dir_name(mod_name))
}

fn path_smapi(game_path: &Path, mod_name: &str) -> PathBuf {
    game_path.join("Mods").join(sanitize_dir_name(mod_name))
}

/// UE pak install path — try to derive the project directory by scanning
/// for `*/Content/Paks/`; otherwise fall back to `<game>/Paks/~mods/`.
///
/// Note: per-mod subfolder is FALSE — pak files merge into `~mods/`.
fn path_ue_paks(game_path: &Path, _mod_name: &str) -> PathBuf {
    if let Some(project_paks) = find_unreal_paks_dir(game_path) {
        return project_paks.join("~mods");
    }
    game_path.join("Paks").join("~mods")
}

fn path_ue4ss(game_path: &Path, mod_name: &str) -> PathBuf {
    // UE4SS lives at `<project>/Binaries/Win64/Mods/`. Try to derive the
    // project from a discovered `Binaries/Win64` directory; else fall back
    // to a per-mod folder under the game root.
    if let Some(win64) = find_unreal_win64_dir(game_path) {
        return win64.join("Mods").join(sanitize_dir_name(mod_name));
    }
    game_path.join(sanitize_dir_name(mod_name))
}

fn path_rimworld(game_path: &Path, mod_name: &str) -> PathBuf {
    game_path.join("Mods").join(sanitize_dir_name(mod_name))
}

/// Sims 4 `.package` install path — `<mods_dir>/<modname>/`.
///
/// Note: the installer passes `data_dir` as `game_path` for plugins where
/// `use_legacy_data_dir() == false`, so `game_path` here is already the
/// resolved `Documents/.../The Sims 4/Mods/` directory.
fn path_sims4_package(mods_dir: &Path, mod_name: &str) -> PathBuf {
    mods_dir.join(sanitize_dir_name(mod_name))
}

/// Sims 4 `.ts4script` install path — directly into `<mods_dir>/`. The
/// game loads scripts by exact filename from the Mods root, so the file
/// lives flat with no per-mod wrapping folder.
fn path_sims4_script(mods_dir: &Path, _mod_name: &str) -> PathBuf {
    mods_dir.to_path_buf()
}

// --- GTA V install-path functions ---

/// GTA V dlcpack install path.
///
/// - Archive shape `dlcpacks/<pack>/dlc.rpf` extracts to the game root and
///   the `dlcpacks/` segment in the archive lays itself out under
///   `<game>/dlcpacks/`. We return the game root for that case.
/// - Archive shape `<pack>/dlc.rpf` (no leading `dlcpacks/`) extracts under
///   `<game>/dlcpacks/` so the pack lands at `<game>/dlcpacks/<pack>/`.
///
/// We can't tell which shape we have here — the install pipeline merges
/// the archive into the returned dir. Defaulting to `<game>/dlcpacks/` is
/// safe for the bare shape and only misroutes the prefixed shape into
/// `<game>/dlcpacks/dlcpacks/<pack>/`. Inspect the archive entries first
/// to disambiguate.
fn path_gtav_dlcpack(game_path: &Path, _mod_name: &str) -> PathBuf {
    game_path.join("dlcpacks")
}

/// ASI / Lua scripts deploy directly to the game root. The archive's own
/// `scripts/` subdir (if present) is preserved automatically by the
/// extraction pass.
fn path_gtav_asi_script(game_path: &Path, _mod_name: &str) -> PathBuf {
    game_path.to_path_buf()
}

/// .NET scripts deploy to the game root; `scripts/` subdir is preserved
/// from the archive layout.
fn path_gtav_net_script(game_path: &Path, _mod_name: &str) -> PathBuf {
    game_path.to_path_buf()
}

/// 3DMigoto/GIMI mods install per-mod under `<game>/Mods/<modname>/`. GIMI
/// (and the sibling SRMI / ZZMI / HIMI loaders) scans `Mods/` recursively
/// and loads each subdirectory's `.ini` + buffer files independently, so
/// each mod gets its own folder.
fn path_gimi(game_path: &Path, mod_name: &str) -> PathBuf {
    game_path.join("Mods").join(sanitize_dir_name(mod_name))
}

/// Mod Engine 2 mods install per-mod under `<game>/mod/<modname>/`.
/// The FromSoft plugin family sets `data_dir` to `<game>/mod`, so when
/// `use_legacy_data_dir() == false` the installer passes that as `game_path`
/// here — meaning we just append the per-mod folder.
fn path_modengine2(mod_dir: &Path, mod_name: &str) -> PathBuf {
    mod_dir.join(sanitize_dir_name(mod_name))
}

fn path_generic(game_path: &Path, mod_name: &str) -> PathBuf {
    game_path.join(sanitize_dir_name(mod_name))
}

/// Walk `game_path` (max depth 4) looking for any directory shaped like
/// `<X>/Content/Paks`, returning that absolute path. Many UE titles bury
/// the project under `<game>/<ProjectName>/Content/Paks/` and the project
/// name is otherwise opaque.
fn find_unreal_paks_dir(game_path: &Path) -> Option<PathBuf> {
    find_dir_path_suffix(game_path, &["Content", "Paks"], 4)
}

/// Same search, for `<X>/Binaries/Win64`. Used by UE4SS Lua mods.
fn find_unreal_win64_dir(game_path: &Path) -> Option<PathBuf> {
    find_dir_path_suffix(game_path, &["Binaries", "Win64"], 4)
}

/// Walk `root` (max `max_depth` levels) and return the first directory
/// whose final path segments equal `suffix` (case-insensitive).
fn find_dir_path_suffix(root: &Path, suffix: &[&str], max_depth: usize) -> Option<PathBuf> {
    fn walk(dir: &Path, suffix: &[&str], depth: usize, max_depth: usize) -> Option<PathBuf> {
        if depth > max_depth {
            return None;
        }
        let entries = std::fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            // Check whether `path` ends with `suffix` (case-insensitive).
            if path_ends_with_ci(&path, suffix) {
                return Some(path);
            }
            if let Some(found) = walk(&path, suffix, depth + 1, max_depth) {
                return Some(found);
            }
        }
        None
    }
    walk(root, suffix, 0, max_depth)
}

fn path_ends_with_ci(path: &Path, suffix: &[&str]) -> bool {
    let comps: Vec<String> = path
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(|s| s.to_lowercase()))
        .collect();
    if comps.len() < suffix.len() {
        return false;
    }
    let start = comps.len() - suffix.len();
    comps[start..]
        .iter()
        .zip(suffix.iter())
        .all(|(a, b)| a == &b.to_lowercase())
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Built-in mod types. Higher priority wins. Iteration order is stable;
/// we sort by priority once at lookup time.
const BUILTIN_MOD_TYPES: &[ModType] = &[
    ModType {
        id: "Generic_BepInExPack_Bootstrap",
        display_name: "BepInExPack (bootstrap)",
        priority: 95,
        detect: detect_bepinex_pack,
        install_path: path_bepinex_pack,
        per_mod_subfolder: false,
    },
    ModType {
        id: "BepInEx",
        display_name: "BepInEx plugin",
        priority: 90,
        detect: detect_bepinex_plugin,
        install_path: path_bepinex_plugin,
        per_mod_subfolder: true,
    },
    ModType {
        id: "SMAPI",
        display_name: "SMAPI (Stardew Valley)",
        priority: 80,
        detect: detect_smapi,
        install_path: path_smapi,
        per_mod_subfolder: true,
    },
    // -- GTA V mod types --
    //
    // Priorities chosen to slot below BepInExPack (95) and above the
    // generic Unreal Engine catch-alls (UE_Paks 70 / UE4SS 60). They fire
    // on file shape only — see the design note above the GTA V detect
    // functions for why namespace overlap with non-GTA games is benign.
    ModType {
        id: "GTAV_DlcPack",
        display_name: "GTA V dlcpack",
        priority: 78,
        detect: detect_gtav_dlcpack,
        install_path: path_gtav_dlcpack,
        per_mod_subfolder: false,
    },
    ModType {
        id: "GTAV_ASIScript",
        display_name: "GTA V ASI / Lua script",
        priority: 72,
        detect: detect_gtav_asi_script,
        install_path: path_gtav_asi_script,
        per_mod_subfolder: false,
    },
    ModType {
        id: "GTAV_NetScript",
        display_name: "GTA V .NET (SHVDN) script",
        priority: 70,
        detect: detect_gtav_net_script,
        install_path: path_gtav_net_script,
        per_mod_subfolder: false,
    },
    ModType {
        id: "ModEngine2",
        display_name: "Mod Engine 2 (FromSoft)",
        // Priority 76 sits between BepInEx (90) / SMAPI (80) — neither would
        // false-positive on a regulation.bin — and the UE / RimWorld /
        // generic types below. Above Sims4_Package (75) and GIMI_Mod (75)
        // since the regulation.bin / FromSoft asset-dir signals are unique
        // to FromSoft titles.
        priority: 76,
        detect: detect_modengine2,
        install_path: path_modengine2,
        per_mod_subfolder: true,
    },
    ModType {
        id: "Sims4_Package",
        display_name: "The Sims 4 .package",
        priority: 75,
        detect: detect_sims4_package,
        install_path: path_sims4_package,
        per_mod_subfolder: true,
    },
    ModType {
        id: "Sims4_Script",
        display_name: "The Sims 4 .ts4script",
        priority: 70,
        detect: detect_sims4_script,
        install_path: path_sims4_script,
        per_mod_subfolder: false,
    },
    ModType {
        id: "GIMI_Mod",
        display_name: "3DMigoto / GIMI mod",
        // Higher than UE_Paks (70) — both could match an archive that ships
        // a `.pak` alongside a `.buf`, but the buffer file is the stronger
        // signal that we're looking at an asset-swap mod. Lower than SMAPI
        // (80) so SMAPI archives that happen to ship a `.buf` payload still
        // route correctly.
        priority: 75,
        detect: detect_gimi,
        install_path: path_gimi,
        per_mod_subfolder: true,
    },
    ModType {
        id: "UE_Paks",
        display_name: "Unreal Engine pak",
        priority: 70,
        detect: detect_ue_paks,
        install_path: path_ue_paks,
        per_mod_subfolder: false,
    },
    ModType {
        id: "UE4SS",
        display_name: "UE4SS Lua mod",
        priority: 60,
        detect: detect_ue4ss,
        install_path: path_ue4ss,
        per_mod_subfolder: true,
    },
    ModType {
        id: "RimWorld",
        display_name: "RimWorld mod",
        priority: 60,
        detect: detect_rimworld,
        install_path: path_rimworld,
        per_mod_subfolder: true,
    },
    ModType {
        id: "Generic_Subfolder",
        display_name: "Generic per-mod subfolder",
        priority: 10,
        detect: detect_generic,
        install_path: path_generic,
        per_mod_subfolder: true,
    },
];

/// Detect the matching mod type for the given archive entry list. Returns
/// the highest-priority type whose `detect` predicate matches. Always
/// succeeds — the `Generic_Subfolder` fallback (priority 10) matches
/// unconditionally.
pub fn detect_mod_type(entries: &[String]) -> Option<&'static ModType> {
    // Sort indices by descending priority and walk in order. Static slice
    // means we can't sort in place; collect references then sort.
    let mut sorted: Vec<&'static ModType> = BUILTIN_MOD_TYPES.iter().collect();
    sorted.sort_by(|a, b| b.priority.cmp(&a.priority));
    sorted.into_iter().find(|mt| (mt.detect)(entries))
}

/// Resolve the absolute deploy target for a mod. Wraps [`detect_mod_type`]
/// to also compute the install path. Always returns a target — the
/// `Generic_Subfolder` fallback ensures we never deploy to the game root
/// for unknown shapes.
pub fn resolve_install_target(
    game_path: &Path,
    mod_name: &str,
    entries: &[String],
) -> InstallTarget {
    let mt = detect_mod_type(entries).expect("Generic_Subfolder always matches");
    let target_dir = (mt.install_path)(game_path, mod_name);
    InstallTarget {
        target_dir,
        per_mod_subfolder: mt.per_mod_subfolder,
        type_id: mt.id,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|s| s.to_string()).collect()
    }

    // --- detect functions ---------------------------------------------------

    #[test]
    fn detect_bepinex_plugin_positive() {
        let e = entries(&["BepInEx/plugins/MyPlugin.dll"]);
        assert!(detect_bepinex_plugin(&e));
    }

    #[test]
    fn detect_bepinex_plugin_loose_plugins() {
        // Some authors strip the BepInEx prefix and ship `plugins/foo.dll`.
        let e = entries(&["plugins/MyPlugin.dll"]);
        assert!(detect_bepinex_plugin(&e));
    }

    #[test]
    fn detect_bepinex_plugin_negative_no_dll() {
        let e = entries(&["BepInEx/plugins/readme.txt"]);
        assert!(!detect_bepinex_plugin(&e));
    }

    #[test]
    fn detect_bepinex_plugin_negative_unrelated() {
        let e = entries(&["meshes/armor/foo.nif", "mymod.esp"]);
        assert!(!detect_bepinex_plugin(&e));
    }

    #[test]
    fn detect_bepinex_pack_positive() {
        let e = entries(&[
            "BepInExPack/winhttp.dll",
            "BepInExPack/BepInEx/core/BepInEx.Core.dll",
        ]);
        assert!(detect_bepinex_pack(&e));
    }

    #[test]
    fn detect_bepinex_pack_negative() {
        let e = entries(&["BepInEx/plugins/foo.dll"]);
        assert!(!detect_bepinex_pack(&e));
    }

    #[test]
    fn detect_smapi_positive() {
        let e = entries(&["MyMod/manifest.json", "MyMod/MyMod.dll"]);
        assert!(detect_smapi(&e));
    }

    #[test]
    fn detect_smapi_loose_manifest() {
        let e = entries(&["manifest.json", "MyMod.dll"]);
        assert!(detect_smapi(&e));
    }

    #[test]
    fn detect_smapi_negative_no_dll() {
        let e = entries(&["MyMod/manifest.json"]);
        assert!(!detect_smapi(&e));
    }

    #[test]
    fn detect_smapi_negative_too_deep() {
        // manifest.json buried at depth 3 — not a SMAPI mod root.
        let e = entries(&["a/b/c/manifest.json", "a/b/c/x.dll"]);
        assert!(!detect_smapi(&e));
    }

    #[test]
    fn detect_ue_paks_positive_top_level() {
        let e = entries(&["MyMod_P.pak"]);
        assert!(detect_ue_paks(&e));
    }

    #[test]
    fn detect_ue_paks_positive_subdir() {
        let e = entries(&["Paks/MyMod_P.pak"]);
        assert!(detect_ue_paks(&e));
    }

    #[test]
    fn detect_ue_paks_negative() {
        let e = entries(&["meshes/foo.nif"]);
        assert!(!detect_ue_paks(&e));
    }

    #[test]
    fn detect_ue4ss_positive_lua() {
        let e = entries(&["Mods/MyMod/Scripts/main.lua"]);
        assert!(detect_ue4ss(&e));
    }

    #[test]
    fn detect_ue4ss_positive_bootstrap() {
        let e = entries(&["dwmapi.dll", "UE4SS-settings.ini", "Mods/shared/enabled.txt"]);
        assert!(detect_ue4ss(&e));
    }

    #[test]
    fn detect_ue4ss_negative() {
        let e = entries(&["BepInEx/plugins/foo.dll"]);
        assert!(!detect_ue4ss(&e));
    }

    #[test]
    fn detect_rimworld_positive_loose() {
        let e = entries(&["About/About.xml", "Assemblies/MyMod.dll"]);
        assert!(detect_rimworld(&e));
    }

    #[test]
    fn detect_rimworld_positive_wrapped() {
        let e = entries(&["MyMod/About/About.xml"]);
        assert!(detect_rimworld(&e));
    }

    #[test]
    fn detect_rimworld_negative() {
        let e = entries(&["mymod.esp"]);
        assert!(!detect_rimworld(&e));
    }

    // --- GIMI / 3DMigoto -------------------------------------------------

    #[test]
    fn detect_gimi_typical_character_mod() {
        // Representative GIMI character swap: per-mod ini + buffer files.
        let e = entries(&[
            "Yelan/Yelan.ini",
            "Yelan/YelanBody.buf",
            "Yelan/YelanBody.ib",
            "Yelan/Body.dds",
        ]);
        assert!(detect_gimi(&e));
    }

    #[test]
    fn detect_gimi_buffer_only_with_ini_at_root() {
        // Some authors flatten the archive with a top-level merged.ini.
        let e = entries(&["merged.ini", "vb1.buf"]);
        assert!(detect_gimi(&e));
    }

    #[test]
    fn detect_gimi_negative_ini_only_no_buffers() {
        // Plain INI archive (e.g. a config patch) — must NOT match GIMI.
        let e = entries(&["Skyrim.ini", "SkyrimPrefs.ini"]);
        assert!(!detect_gimi(&e));
    }

    #[test]
    fn detect_gimi_negative_buffers_only_no_ini() {
        // Buffer files without an INI shouldn't match — GIMI requires the
        // INI to define the hash/draw rules.
        let e = entries(&["raw.buf", "data.ib"]);
        assert!(!detect_gimi(&e));
    }

    #[test]
    fn detect_gimi_case_insensitive_extensions() {
        let e = entries(&["Char/Char.INI", "Char/MeshA.BUF"]);
        assert!(detect_gimi(&e));
    }

    #[test]
    fn detect_generic_always_matches() {
        assert!(detect_generic(&entries(&[])));
        assert!(detect_generic(&entries(&["random.txt"])));
    }

    #[test]
    fn detect_sims4_package_positive() {
        let e = entries(&["MyHair.package"]);
        assert!(detect_sims4_package(&e));
    }

    #[test]
    fn detect_sims4_package_nested() {
        let e = entries(&["Author/CoolMod/awesome.package", "Author/CoolMod/readme.txt"]);
        assert!(detect_sims4_package(&e));
    }

    #[test]
    fn detect_sims4_package_negative() {
        let e = entries(&["MyMod_P.pak"]);
        assert!(!detect_sims4_package(&e));
    }

    #[test]
    fn detect_sims4_script_positive() {
        let e = entries(&["mc_command_center.ts4script"]);
        assert!(detect_sims4_script(&e));
    }

    #[test]
    fn detect_sims4_script_nested() {
        let e = entries(&["MCCC/mc_woohoo.ts4script"]);
        assert!(detect_sims4_script(&e));
    }

    #[test]
    fn detect_sims4_script_negative() {
        let e = entries(&["plugin.dll"]);
        assert!(!detect_sims4_script(&e));
    }

    // --- detect_mod_type priority ordering ---------------------------------

    #[test]
    fn priority_bepinex_pack_beats_plugin() {
        // The pack ships `BepInExPack/...` AND `BepInEx/plugins/...` — the
        // pack should win because of higher priority.
        let e = entries(&[
            "BepInExPack/winhttp.dll",
            "BepInExPack/BepInEx/plugins/Core.dll",
            "BepInEx/plugins/Helper.dll",
        ]);
        let mt = detect_mod_type(&e).unwrap();
        assert_eq!(mt.id, "Generic_BepInExPack_Bootstrap");
    }

    #[test]
    fn priority_bepinex_beats_generic() {
        let e = entries(&["BepInEx/plugins/Foo.dll", "readme.txt"]);
        let mt = detect_mod_type(&e).unwrap();
        assert_eq!(mt.id, "BepInEx");
    }

    #[test]
    fn priority_unknown_falls_back_to_generic() {
        let e = entries(&["random.txt", "subdir/another.txt"]);
        let mt = detect_mod_type(&e).unwrap();
        assert_eq!(mt.id, "Generic_Subfolder");
    }

    // --- resolve_install_target --------------------------------------------

    #[test]
    fn resolve_bepinex_plugin_target() {
        let game = Path::new("/game/MyGame");
        let e = entries(&["BepInEx/plugins/Foo.dll"]);
        let target = resolve_install_target(game, "Cool Plugin / v1.0", &e);
        assert_eq!(target.type_id, "BepInEx");
        assert!(target.per_mod_subfolder);
        // Sanitized: `/` -> `_`
        assert_eq!(
            target.target_dir,
            Path::new("/game/MyGame/BepInEx/plugins/Cool Plugin _ v1.0")
        );
    }

    #[test]
    fn resolve_smapi_target() {
        let game = Path::new("/game/Stardew");
        let e = entries(&["manifest.json", "MyMod.dll"]);
        let target = resolve_install_target(game, "MyMod", &e);
        assert_eq!(target.type_id, "SMAPI");
        assert!(target.per_mod_subfolder);
        assert_eq!(target.target_dir, Path::new("/game/Stardew/Mods/MyMod"));
    }

    #[test]
    fn resolve_ue_paks_fallback_when_no_project() {
        // No `<X>/Content/Paks/` discoverable — fall back to <game>/Paks/~mods/.
        let game = Path::new("/nonexistent/game");
        let e = entries(&["MyMod_P.pak"]);
        let target = resolve_install_target(game, "MyMod", &e);
        assert_eq!(target.type_id, "UE_Paks");
        assert!(!target.per_mod_subfolder);
        assert_eq!(target.target_dir, Path::new("/nonexistent/game/Paks/~mods"));
    }

    #[test]
    fn resolve_ue_paks_uses_project_dir_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().to_path_buf();
        let project_paks = game.join("MyProject").join("Content").join("Paks");
        std::fs::create_dir_all(&project_paks).unwrap();

        let e = entries(&["MyMod_P.pak"]);
        let target = resolve_install_target(&game, "MyMod", &e);
        assert_eq!(target.type_id, "UE_Paks");
        assert_eq!(target.target_dir, project_paks.join("~mods"));
    }

    #[test]
    fn resolve_gimi_target() {
        let game = Path::new("/game/Genshin Impact game");
        let e = entries(&[
            "YelanRework/YelanRework.ini",
            "YelanRework/Body.buf",
            "YelanRework/Body.ib",
        ]);
        let target = resolve_install_target(game, "Yelan Rework", &e);
        assert_eq!(target.type_id, "GIMI_Mod");
        assert!(target.per_mod_subfolder);
        assert_eq!(
            target.target_dir,
            Path::new("/game/Genshin Impact game/Mods/Yelan Rework")
        );
    }

    #[test]
    fn priority_gimi_beats_ue_paks_when_both_match() {
        // Hypothetical archive shipping both a pak and a buffer/ini — GIMI
        // should win because of higher priority (and because authors
        // shipping buffers really mean a 3DMigoto mod, not a UE pak mod).
        let e = entries(&["MyMod/MyMod.ini", "MyMod/MyMod.buf", "Bundle.pak"]);
        let mt = detect_mod_type(&e).unwrap();
        assert_eq!(mt.id, "GIMI_Mod");
    }

    #[test]
    fn resolve_rimworld_target() {
        let game = Path::new("/game/RimWorld");
        let e = entries(&["About/About.xml", "Assemblies/Mod.dll"]);
        let target = resolve_install_target(game, "MyRimMod", &e);
        assert_eq!(target.type_id, "RimWorld");
        assert!(target.per_mod_subfolder);
        assert_eq!(target.target_dir, Path::new("/game/RimWorld/Mods/MyRimMod"));
    }

    #[test]
    fn resolve_sims4_package_target() {
        // For Sims 4, the install pipeline passes the resolved Mods directory
        // as `game_path` (because the plugin returns `use_legacy_data_dir =
        // false` and its data_dir lives outside the game install).
        let mods_dir = Path::new("/Documents/Electronic Arts/The Sims 4/Mods");
        let e = entries(&["Hair_Pack/curly.package", "Hair_Pack/wavy.package"]);
        let target = resolve_install_target(mods_dir, "Hair Pack", &e);
        assert_eq!(target.type_id, "Sims4_Package");
        assert!(target.per_mod_subfolder);
        assert_eq!(
            target.target_dir,
            Path::new("/Documents/Electronic Arts/The Sims 4/Mods/Hair Pack")
        );
    }

    #[test]
    fn resolve_sims4_script_target_no_subfolder() {
        let mods_dir = Path::new("/Documents/Electronic Arts/The Sims 4/Mods");
        let e = entries(&["mc_command_center.ts4script"]);
        let target = resolve_install_target(mods_dir, "MCCC", &e);
        assert_eq!(target.type_id, "Sims4_Script");
        assert!(!target.per_mod_subfolder);
        // Script files install flat under the Mods root.
        assert_eq!(target.target_dir, mods_dir);
    }

    #[test]
    fn resolve_sims4_package_beats_generic() {
        // `.package` files must route to Sims4_Package, not Generic_Subfolder.
        let mods_dir = Path::new("/Mods");
        let e = entries(&["readme.txt", "skin.package"]);
        let target = resolve_install_target(mods_dir, "Skin Mod", &e);
        assert_eq!(target.type_id, "Sims4_Package");
    }

    #[test]
    fn resolve_sims4_package_does_not_fire_on_pak() {
        // UE pak files have extension "pak" — distinct from "package".
        // A `.pak` archive must NOT route to Sims4_Package.
        let mods_dir = Path::new("/Mods");
        let e = entries(&["MyTextures_P.pak"]);
        let target = resolve_install_target(mods_dir, "Textures", &e);
        assert_eq!(target.type_id, "UE_Paks");
    }

    #[test]
    fn resolve_generic_fallback_target() {
        let game = Path::new("/game/Mystery");
        let e = entries(&["readme.txt", "data/whatever.bin"]);
        let target = resolve_install_target(game, "Some Mod", &e);
        assert_eq!(target.type_id, "Generic_Subfolder");
        assert!(target.per_mod_subfolder);
        assert_eq!(target.target_dir, Path::new("/game/Mystery/Some Mod"));
    }

    #[test]
    fn resolve_bepinex_pack_extracts_to_game_root() {
        let game = Path::new("/game/Valheim");
        let e = entries(&["BepInExPack/winhttp.dll", "BepInExPack/BepInEx/core/x.dll"]);
        let target = resolve_install_target(game, "BepInExPack", &e);
        assert_eq!(target.type_id, "Generic_BepInExPack_Bootstrap");
        assert!(!target.per_mod_subfolder);
        assert_eq!(target.target_dir, game);
    }

    // --- sanitize_dir_name -------------------------------------------------

    #[test]
    fn sanitize_strips_invalid_chars() {
        assert_eq!(sanitize_dir_name("foo/bar:baz"), "foo_bar_baz");
        assert_eq!(sanitize_dir_name("a*b?c|d"), "a_b_c_d");
    }

    #[test]
    fn sanitize_trims_dots_and_whitespace() {
        assert_eq!(sanitize_dir_name(" .MyMod. "), "MyMod");
    }

    #[test]
    fn sanitize_empty_falls_back() {
        assert_eq!(sanitize_dir_name(""), "mod");
        assert_eq!(sanitize_dir_name("..."), "mod");
        assert_eq!(sanitize_dir_name("   "), "mod");
    }

    // --- GTA V detect tests -------------------------------------------------

    #[test]
    fn detect_gtav_dlcpack_with_prefix() {
        let e = entries(&["dlcpacks/myDLC/dlc.rpf"]);
        assert!(detect_gtav_dlcpack(&e));
    }

    #[test]
    fn detect_gtav_dlcpack_bare_shape() {
        let e = entries(&["myDLC/dlc.rpf"]);
        assert!(detect_gtav_dlcpack(&e));
    }

    #[test]
    fn detect_gtav_dlcpack_negative() {
        let e = entries(&["foo.asi", "scripts/main.lua"]);
        assert!(!detect_gtav_dlcpack(&e));
        // Just a top-level dlc.rpf with no folder is not a valid pack.
        let e = entries(&["dlc.rpf"]);
        assert!(!detect_gtav_dlcpack(&e));
    }

    #[test]
    fn detect_gtav_asi_positive() {
        let e = entries(&["MyMod.asi"]);
        assert!(detect_gtav_asi_script(&e));
    }

    #[test]
    fn detect_gtav_asi_lua_addins() {
        let e = entries(&["scripts/addins/menyoo.lua"]);
        assert!(detect_gtav_asi_script(&e));
    }

    #[test]
    fn detect_gtav_asi_negative() {
        let e = entries(&["BepInEx/plugins/foo.dll"]);
        assert!(!detect_gtav_asi_script(&e));
    }

    #[test]
    fn detect_gtav_net_via_shvdn_reference() {
        let e = entries(&["ScriptHookVDotNet.asi", "ScriptHookVDotNet3.dll"]);
        assert!(detect_gtav_net_script(&e));
    }

    #[test]
    fn detect_gtav_net_via_scripts_dir() {
        let e = entries(&["scripts/MyModScript.dll"]);
        assert!(detect_gtav_net_script(&e));
    }

    #[test]
    fn detect_gtav_net_negative_loose_dll() {
        // Plain `foo.dll` at root is NOT a SHVDN script — could be anything.
        let e = entries(&["foo.dll", "readme.txt"]);
        assert!(!detect_gtav_net_script(&e));
    }

    #[test]
    fn priority_dlcpack_beats_asi_when_both_present() {
        // Hybrid release — some packs ship both dlcpacks/ and an ASI
        // helper. dlcpack (78) should win over ASIScript (72).
        let e = entries(&[
            "dlcpacks/myPack/dlc.rpf",
            "MyHelper.asi",
        ]);
        let mt = detect_mod_type(&e).unwrap();
        assert_eq!(mt.id, "GTAV_DlcPack");
    }

    #[test]
    fn priority_asi_beats_net_when_both_present() {
        // ASIScript (72) > NetScript (70). A pack with both .asi and a
        // SHVDN .dll lands as ASIScript.
        let e = entries(&["Something.asi", "scripts/Helper.dll"]);
        let mt = detect_mod_type(&e).unwrap();
        assert_eq!(mt.id, "GTAV_ASIScript");
    }

    #[test]
    fn resolve_gtav_dlcpack_target() {
        let game = Path::new("/game/GTAV");
        let e = entries(&["dlcpacks/myPack/dlc.rpf"]);
        let target = resolve_install_target(game, "MyPack", &e);
        assert_eq!(target.type_id, "GTAV_DlcPack");
        assert!(!target.per_mod_subfolder);
        assert_eq!(target.target_dir, Path::new("/game/GTAV/dlcpacks"));
    }

    #[test]
    fn resolve_gtav_asi_to_root() {
        let game = Path::new("/game/GTAV");
        let e = entries(&["NativeTrainer.asi"]);
        let target = resolve_install_target(game, "Native Trainer", &e);
        assert_eq!(target.type_id, "GTAV_ASIScript");
        assert!(!target.per_mod_subfolder);
        assert_eq!(target.target_dir, Path::new("/game/GTAV"));
    }

    #[test]
    fn resolve_gtav_net_to_root() {
        let game = Path::new("/game/GTAV");
        let e = entries(&["scripts/MyMod.dll"]);
        let target = resolve_install_target(game, "MyMod", &e);
        assert_eq!(target.type_id, "GTAV_NetScript");
        assert!(!target.per_mod_subfolder);
        assert_eq!(target.target_dir, Path::new("/game/GTAV"));
    }

    // --- Mod Engine 2 (FromSoft) ------------------------------------------

    #[test]
    fn detect_modengine2_regulation_only() {
        // Single-file regulation.bin shipped under a wrapper folder is the
        // most common Elden Ring / DS3 mod shape.
        let e = entries(&["MyMod/regulation.bin"]);
        assert!(detect_modengine2(&e));
    }

    #[test]
    fn detect_modengine2_parts_dir() {
        // Weapon/armor model swap — parts/ asset dir under a wrapper folder.
        let e = entries(&["MyMod/parts/wp_a_0001.bnd"]);
        assert!(detect_modengine2(&e));
    }

    #[test]
    fn detect_modengine2_param_dir() {
        // Top-level param/ — gameplay table override.
        let e = entries(&["param/equipparamweapon.param"]);
        assert!(detect_modengine2(&e));
    }

    #[test]
    fn detect_modengine2_toml() {
        // Some mods bundle a modengine2.toml config alongside payloads.
        let e = entries(&["MyMod/modengine2.toml"]);
        assert!(detect_modengine2(&e));
    }

    #[test]
    fn detect_modengine2_no_match_for_bepinex() {
        // BepInEx archive must NOT match Mod Engine 2 — keeps priority
        // ordering safe even on shape overlap.
        let e = entries(&["BepInEx/plugins/MyPlugin.dll"]);
        assert!(!detect_modengine2(&e));
    }

    #[test]
    fn detect_modengine2_path_resolution() {
        // path_modengine2 is just `<mod_dir>/<sanitized name>` — the
        // FromSoft plugin family will pass `<game>/mod` as the first arg.
        let mod_dir = Path::new("/game/EldenRing/mod");
        let result = path_modengine2(mod_dir, "MyMod");
        assert_eq!(result, Path::new("/game/EldenRing/mod/MyMod"));
    }

    #[test]
    fn resolve_modengine2_picks_over_generic() {
        // A bare `regulation.bin` archive must route to ModEngine2, not
        // Generic_Subfolder.
        let mod_dir = Path::new("/game/EldenRing/mod");
        let e = entries(&["regulation.bin"]);
        let target = resolve_install_target(mod_dir, "Reforged", &e);
        assert_eq!(target.type_id, "ModEngine2");
        assert!(target.per_mod_subfolder);
        assert_eq!(target.target_dir, Path::new("/game/EldenRing/mod/Reforged"));
    }
}
