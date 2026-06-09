//! Paralives (native macOS) game plugin.
//!
//! Paralives is a Unity (IL2CPP) life-sim by Paralives Studio with first-
//! class official mod support: feature mods, asset mods, and a Steam
//! Workshop integration. Mods drop into the Unity persistent data path
//! at `~/Library/Application Support/com.Paralives.Paralives/Mods/`,
//! which is OUTSIDE the `.app` bundle — bundle code signing is preserved.
//!
//! ## Supported mod formats
//!
//! **Data mods** (all cross-platform, data-only):
//! `.fbx`, `.obj`, `.png`, `.jpg`, `.jpeg`, `.catalog`, `.ogg`, `.wav`,
//! `.json`, `.ttf` — deployed to the Unity persistent data Mods/ path.
//!
//! **BepInEx plugin mods** (macOS ARM64 IL2CPP, community runtime):
//! `.dll` files located under a `BepInEx/plugins/` path in the archive, or
//! at the staging root (early/simple BepInEx mod archives). Deployed to
//! `<game_install>/BepInEx/plugins/<mod_db_id>/`. Requires BepInEx 6.x
//! IL2CPP macOS ARM64 to be installed first (Layer 1 detection; Layer 3
//! install).
//!
//! ## BepInEx deploy guard
//!
//! If a staged mod contains BepInEx plugin files but
//! `paralives_bepinex::detect()` reports `installed: false`, deploy returns
//! a typed error directing the user to Settings → Native. If BepInEx is
//! present but `mac_supported: false` (x86_64 / BepInEx 5.x), deploy also
//! refuses with a message naming the ARM64 6.x requirement.
//!
//! ## Rejected artifacts
//!
//! - `.exe` — Windows executables
//! - `winhttp.dll` — Windows-flavor BepInEx Doorstop hook
//! - `doorstop_config.ini` — BepInEx Doorstop config (ships with the BepInEx
//!   install itself, not individual mods)
//!
//! Note: plain `.dll` files are no longer unconditionally refused. They are
//! classified as BepInEx plugin candidates and routed accordingly.
//!
//! Apple Silicon only (ARM64 native). Steam App ID 1118520.
//! CFBundleIdentifier best guess: `com.Paralives.Paralives` — TODO verify
//! with a real install by reading
//! `Paralives.app/Contents/Info.plist` CFBundleIdentifier.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::bottles::Bottle;
use crate::database::ModDatabase;
use crate::deployer::{DeployResult, DeployerError};
use crate::games::{DetectedGame, GamePlugin, LoadOrderKind};
use crate::staging::is_safe_relative_path;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// CFBundleIdentifier for Paralives.
///
/// Best guess derived from Unity's `Application.persistentDataPath` formula
/// (`com.<CompanyName>.<ProductName>`) and community-confirmed path at
/// `~/Library/Application Support/com.Paralives.Paralives/`.
/// TODO: verify with a real install.
const PARALIVES_BUNDLE_IDENTIFIER: &str = "com.Paralives.Paralives";

/// Executable name inside `Paralives.app/Contents/MacOS/`.
/// Inferred from Unity's ProductName convention; TODO verify with real install.
const PARALIVES_BUNDLE_EXECUTABLE: &str = "Paralives";

/// Steam App ID for Paralives.
const PARALIVES_STEAM_APP_ID: &str = "1118520";

/// Bottle sentinel for native mods (no Wine bottle).
const PARALIVES_NATIVE_BOTTLE_SENTINEL: &str = "";

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

/// Game plugin for Paralives (native macOS).
///
/// Deploys data mods to the Unity persistent data path and BepInEx plugin
/// mods to `<game_install>/BepInEx/plugins/<mod_db_id>/`. The .app bundle
/// is never touched; code signing is preserved.
pub struct ParalivesNativePlugin;

// ---------------------------------------------------------------------------
// Path resolution helpers
// ---------------------------------------------------------------------------

/// Returns the Paralives mods directory for the current user.
///
/// Resolves to `~/Library/Application Support/com.Paralives.Paralives/Mods/`
/// via `dirs::home_dir()`. The path is returned even if it does not yet exist
/// on disk — callers create it before use.
///
/// TODO: verify with a real install that this path is the one Paralives
/// actually uses. The in-game Mods button opens the folder on disk — check
/// which path it points to on first launch.
pub fn resolve_mods_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    home.join("Library/Application Support/com.Paralives.Paralives/Mods")
}

/// Returns the game install directory (parent of the `.app` bundle) from a
/// `DetectedGame` with a native runtime.
///
/// For a Steam install this is typically
/// `Steam/steamapps/common/Paralives/`. Returns `None` if the runtime is not
/// native or the bundle path has no parent.
pub fn resolve_game_install_dir(detected: &DetectedGame) -> Option<PathBuf> {
    detected
        .runtime
        .native()
        .and_then(|n| n.app_bundle_path.parent().map(|p| p.to_path_buf()))
}

// ---------------------------------------------------------------------------
// Detection helper (pure function for testability)
// ---------------------------------------------------------------------------

/// Filter `native_scanner` candidates to Paralives installs and produce
/// `DetectedGame` entries. Pure function — the `GamePlugin::detect_native`
/// impl wraps this with the actual scanner call.
///
/// Accepts candidates matching:
/// - `bundle_identifier == "com.Paralives.Paralives"` (primary), OR
/// - `bundle_executable == "Paralives"` (fallback for non-standard packaging).
///
/// Sandboxed candidates are always rejected.
fn detect_from_candidates(
    candidates: Vec<crate::native_scanner::NativeAppCandidate>,
) -> Vec<DetectedGame> {
    candidates
        .into_iter()
        .filter(|c| !c.sandboxed)
        .filter(|c| {
            c.info.bundle_identifier == PARALIVES_BUNDLE_IDENTIFIER
                || c.info.bundle_executable == PARALIVES_BUNDLE_EXECUTABLE
        })
        .map(|c| {
            let game_path = c.bundle_path.join("Contents").join("MacOS");
            let exe_path = Some(game_path.join("Paralives"));
            let data_dir = resolve_mods_dir();
            let steam_app_id = if c.source == crate::runtime::NativeSource::Steam {
                Some(PARALIVES_STEAM_APP_ID.to_string())
            } else {
                None
            };
            DetectedGame {
                game_id: "paralives_native".to_string(),
                display_name: "Paralives".to_string(),
                nexus_slug: "paralives".to_string(),
                game_path: game_path.clone(),
                exe_path,
                data_dir,
                runtime: crate::runtime::GameRuntime::Native(crate::runtime::NativeContext {
                    app_bundle_path: c.bundle_path,
                    game_data_root: game_path,
                    architecture: c.architecture,
                    sandboxed: false,
                    source: c.source,
                }),
                steam_app_id,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// File classification
// ---------------------------------------------------------------------------

/// Outcome of classifying all staged files for a single mod.
struct ClassifiedFiles {
    /// Files that should be deployed into the BepInEx plugins directory.
    /// Each entry is `(absolute_path, relative_path_from_staging_root)`.
    bepinex_plugin_files: Vec<(PathBuf, String)>,
    /// Files that should be deployed into the Paralives data Mods/ directory.
    data_mod_files: Vec<(PathBuf, String)>,
}

/// Classify every `(absolute_path, relative_path)` tuple in `staged` into
/// either a BepInEx plugin file or a data mod file.
///
/// ## BepInEx plugin classification rules (in priority order)
///
/// 1. Relative path contains a `bepinex/plugins/` segment (case-insensitive,
///    `/`-normalised) — archive author structured the mod correctly.
/// 2. Relative path starts with `bepinex/plugins/` (top-level variant of 1).
/// 3. No path separator and extension is `.dll` — top-level `.dll`, the
///    "early BepInEx mod" pattern where the author just shipped the assembly
///    without nesting it in the BepInEx layout.
///
/// All other files (`.dll` under non-plugins subdirs, data formats, etc.) go
/// to the data bucket. The `rejects_paralives_artifact` predicate is applied
/// to data files at deploy time.
fn classify_files(staged: &[(PathBuf, String)]) -> ClassifiedFiles {
    let mut out = ClassifiedFiles {
        bepinex_plugin_files: Vec::new(),
        data_mod_files: Vec::new(),
    };
    for entry in staged {
        let rel_lower = entry.1.replace('\\', "/").to_lowercase();
        let is_bepinex = rel_lower.contains("/bepinex/plugins/")
            || rel_lower.starts_with("bepinex/plugins/")
            || (!rel_lower.contains('/') && rel_lower.ends_with(".dll"));
        if is_bepinex {
            out.bepinex_plugin_files.push(entry.clone());
        } else {
            out.data_mod_files.push(entry.clone());
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Artifact rejection predicate
// ---------------------------------------------------------------------------

/// Returns `true` if the relative file path is a Windows-only loader or other
/// artifact that the deploy pipeline cannot handle.
///
/// Rejected patterns:
/// - `.exe` — Windows executables
/// - `winhttp.dll` — the BepInEx Doorstop hook DLL (Windows-specific loader)
/// - `doorstop_config.ini` — BepInEx Doorstop configuration (belongs to the
///   BepInEx install itself, not individual mods)
///
/// Note: plain `.dll` is NOT rejected here. DLLs are BepInEx plugin
/// candidates and are classified by [`classify_files`] before reaching this
/// predicate. Only the two Windows-loader specific names are special-cased.
pub fn rejects_paralives_artifact(rel_path: &str) -> bool {
    let lower = rel_path.replace('\\', "/").to_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);

    // Windows executables — always rejected.
    if name.ends_with(".exe") {
        return true;
    }
    // Windows-flavor BepInEx Doorstop hook — wrong OS.
    if name == "winhttp.dll" {
        return true;
    }
    // BepInEx Doorstop config — belongs to the BepInEx install, not the mod.
    if name == "doorstop_config.ini" {
        return true;
    }

    false
}

// ---------------------------------------------------------------------------
// Single-file deploy helper
// ---------------------------------------------------------------------------

/// Copy or hardlink a single file from `abs` to `dest_root/rel`.
///
/// Creates parent directories as needed. Performs a post-join canonicalization
/// check to confirm the destination stays inside `canonical_dest_root`.
/// Removes any existing file at the destination so that `hard_link` does not
/// fail with EEXIST.
fn deploy_one_file(
    abs: &Path,
    rel: &str,
    dest_root: &Path,
    canonical_dest_root: &Path,
) -> Result<(), DeployerError> {
    let dest = dest_root.join(rel);
    let dest_parent = dest.parent().unwrap_or(dest_root);
    std::fs::create_dir_all(dest_parent)
        .map_err(|e| DeployerError::Other(format!("create dest parent: {}", e)))?;
    let canonical_parent = dest_parent
        .canonicalize()
        .unwrap_or_else(|_| dest_parent.to_path_buf());
    if !canonical_parent.starts_with(canonical_dest_root) {
        return Err(DeployerError::Other(format!(
            "destination escapes deploy root: {}",
            dest.display()
        )));
    }
    if dest.exists() {
        let _ = std::fs::remove_file(&dest);
    }
    if std::fs::hard_link(abs, &dest).is_err() {
        std::fs::copy(abs, &dest)
            .map_err(|e| DeployerError::Other(format!("copy mod file: {}", e)))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Inner (testable) deploy function
// ---------------------------------------------------------------------------

/// Core deployment logic for Paralives native mods.
///
/// Takes explicit `mods_dir` and `game_install_dir` arguments (rather than
/// resolving from HOME / `detected`) so that unit tests can redirect to
/// tempdirs.
///
/// ## Algorithm
///
/// 1. Verify `detected.runtime` is [`crate::runtime::GameRuntime::Native`]
///    and the game is not sandboxed.
/// 2. Create `mods_dir` if absent.
/// 3. Snapshot the current state (best-effort).
/// 4. Walk enabled mods; for each mod collect all staged files and classify
///    them with [`classify_files`]:
///    - **BepInEx plugin files**: verify BepInEx is installed and
///      `mac_supported`, then hardlink/copy into
///      `<game_install_dir>/BepInEx/plugins/<mod_db_id>/`.
///    - **Data mod files**: apply [`rejects_paralives_artifact`], then
///      hardlink/copy into `mods_dir/<relative_path>`.
/// 5. Return [`DeployResult`] with the deployed file count.
pub fn deploy_native_inner(
    detected: &DetectedGame,
    db: &Arc<ModDatabase>,
    mods_dir: &Path,
    game_install_dir: Option<&Path>,
) -> Result<DeployResult, DeployerError> {
    // 1. Reject non-native and sandboxed games.
    let native = detected
        .runtime
        .native()
        .ok_or_else(|| DeployerError::Other("expected native runtime for Paralives deploy".into()))?;
    if native.sandboxed {
        return Err(DeployerError::Other(format!(
            "native modding refused for sandboxed app: {}",
            native.app_bundle_path.display()
        )));
    }

    // 2. Create the data mods directory.
    std::fs::create_dir_all(mods_dir)
        .map_err(|e| DeployerError::Other(format!("create mods_dir: {}", e)))?;

    // 3. Pre-deploy snapshot (best-effort — failure must not abort deploy).
    if let Err(e) = crate::rollback::create_native_snapshot(
        db,
        &detected.game_id,
        "paralives-deploy",
        &format!("Paralives deploy to {}", mods_dir.display()),
    ) {
        log::warn!("snapshot before Paralives deploy failed: {}", e);
    }

    // 4. Walk enabled mods from the database.
    let enabled_mods = db
        .list_mods(&detected.game_id, PARALIVES_NATIVE_BOTTLE_SENTINEL)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    // Canonicalise mods_dir once for destination-escape checks (data side).
    let canonical_mods_dir = mods_dir
        .canonicalize()
        .unwrap_or_else(|_| mods_dir.to_path_buf());

    let mut deployed_count = 0usize;

    for installed_mod in enabled_mods.iter().filter(|m| m.enabled) {
        let staging_dir = match &installed_mod.staging_path {
            Some(p) => PathBuf::from(p),
            None => {
                log::warn!(
                    "paralives deploy_native: mod '{}' has no staging path, skipping",
                    installed_mod.name
                );
                continue;
            }
        };

        if !staging_dir.exists() {
            log::warn!(
                "paralives deploy_native: staging dir missing for mod '{}': {}",
                installed_mod.name,
                staging_dir.display()
            );
            continue;
        }

        // Collect all files (absolute_path, relative_path) for this mod.
        let staged: Vec<(PathBuf, String)> = walkdir::WalkDir::new(&staging_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| {
                let abs = e.into_path();
                let rel = abs
                    .strip_prefix(&staging_dir)
                    .ok()?
                    .to_string_lossy()
                    .to_string();
                Some((abs, rel))
            })
            .collect();

        let classified = classify_files(&staged);

        // ── BepInEx plugin files ─────────────────────────────────────────────
        if !classified.bepinex_plugin_files.is_empty() {
            let install_dir = game_install_dir.ok_or_else(|| {
                DeployerError::Other(
                    "BepInEx plugin staged but game install directory could not be resolved"
                        .into(),
                )
            })?;

            let bepinex_status = crate::paralives_bepinex::detect(install_dir);

            if !bepinex_status.installed {
                return Err(DeployerError::Other(format!(
                    "BepInEx is required to use the mod '{}' but is not installed. \
                     Install BepInEx via Settings \u{2192} Native or manually before deploying.",
                    installed_mod.name
                )));
            }
            if !bepinex_status.mac_supported {
                return Err(DeployerError::Other(format!(
                    "BepInEx is installed but the wrong architecture for Apple Silicon \
                     (likely BepInEx 5.x). Install BepInEx 6.x IL2CPP macOS ARM64 to use '{}'.",
                    installed_mod.name
                )));
            }

            // Deploy into <install_dir>/BepInEx/plugins/<mod_db_id>/.
            // Use the numeric DB id as the subdirectory name — it is unique,
            // filesystem-safe, and stable across renames.
            let plugin_root = install_dir
                .join("BepInEx")
                .join("plugins")
                .join(installed_mod.id.to_string());
            std::fs::create_dir_all(&plugin_root)
                .map_err(|e| DeployerError::Other(format!("create plugin_root: {}", e)))?;
            let canonical_plugin_root = plugin_root
                .canonicalize()
                .unwrap_or_else(|_| plugin_root.clone());

            for (abs, rel) in &classified.bepinex_plugin_files {
                // Path safety: the relative paths come from walkdir over our
                // own staging dir, but validate anyway.
                if !is_safe_relative_path(rel) {
                    return Err(DeployerError::Other(format!(
                        "unsafe BepInEx plugin path in mod '{}': {}",
                        installed_mod.name, rel
                    )));
                }
                // Strip any leading BepInEx/plugins/ prefix so files land
                // directly inside plugin_root rather than being double-nested.
                let dest_rel = strip_bepinex_plugins_prefix(rel);
                deploy_one_file(abs, dest_rel, &plugin_root, &canonical_plugin_root)?;
                deployed_count += 1;
            }
        }

        // ── Data mod files ───────────────────────────────────────────────────
        for (abs, rel) in &classified.data_mod_files {
            // Reject Windows-only loaders and other invalid artifacts.
            if rejects_paralives_artifact(rel) {
                return Err(DeployerError::Other(format!(
                    "unsupported Paralives artifact (Windows-only or invalid) \
                     in mod '{}': {}",
                    installed_mod.name, rel
                )));
            }

            // Validate relative path safety (no traversal, null bytes, drive letters).
            if !is_safe_relative_path(rel) {
                return Err(DeployerError::Other(format!(
                    "unsafe mod path in mod '{}': {}",
                    installed_mod.name, rel
                )));
            }

            deploy_one_file(abs, rel, mods_dir, &canonical_mods_dir)?;
            deployed_count += 1;
        }
    }

    Ok(DeployResult {
        deployed_count,
        skipped_count: 0,
        fallback_used: false,
    })
}

/// Strip a leading `BepInEx/plugins/` (case-insensitive, normalised)
/// prefix from a relative path so that plugin DLLs land directly inside
/// `plugin_root` rather than being double-nested under `BepInEx/plugins/`.
///
/// For top-level DLLs (no separator) the path is returned unchanged.
fn strip_bepinex_plugins_prefix(rel: &str) -> &str {
    let norm = rel.replace('\\', "/");
    // Lowercase comparison only — the original rel is what we return sliced.
    let lower = norm.to_lowercase();
    for prefix in &["bepinex/plugins/"] {
        if lower.starts_with(prefix) {
            // Return the part after the prefix, referencing the original bytes.
            return &rel[prefix.len()..];
        }
        // Also handle an interior segment (e.g. "SubDir/BepInEx/plugins/Mod.dll")
        // by finding the first occurrence.
        if let Some(pos) = lower.find(&format!("/{}", prefix)) {
            return &rel[pos + 1 + prefix.len()..];
        }
    }
    rel
}

// ---------------------------------------------------------------------------
// GamePlugin impl
// ---------------------------------------------------------------------------

impl GamePlugin for ParalivesNativePlugin {
    fn game_id(&self) -> &str {
        "paralives_native"
    }

    fn display_name(&self) -> &str {
        "Paralives (Native)"
    }

    fn nexus_slug(&self) -> &str {
        "paralives"
    }

    fn executables(&self) -> &[&str] {
        &[PARALIVES_BUNDLE_EXECUTABLE]
    }

    fn detect_wine(&self, _bottle: &Bottle) -> Option<DetectedGame> {
        // Paralives is Apple Silicon-only; Wine/CrossOver not applicable.
        None
    }

    fn detect_native(&self) -> Vec<DetectedGame> {
        detect_from_candidates(crate::native_scanner::scan_all_native())
    }

    /// Returns the mods directory for Paralives.
    ///
    /// Note: the true Paralives mods directory is at the Unity persistent data
    /// path (`~/Library/Application Support/com.Paralives.Paralives/Mods/`),
    /// independent of the .app bundle location. Use [`resolve_mods_dir`] for
    /// deployment; this method is used by the generic mod install pipeline.
    fn get_data_dir(&self, _game_path: &Path) -> PathBuf {
        resolve_mods_dir()
    }

    /// Paralives has no `plugins.txt`-style load order file.
    ///
    /// Mod activation is managed by the in-game Mods menu; Corkscrew's role
    /// is file deployment only.
    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        None
    }

    /// Paralives has no load order — data mods are not ordered.
    fn load_order_kind(&self, _game_path: &Path) -> LoadOrderKind {
        LoadOrderKind::None
    }

    /// Deploy all staged Paralives mods.
    ///
    /// Data mods go to the Unity persistent data Mods/ path. BepInEx plugin
    /// mods go to `<game_install>/BepInEx/plugins/<mod_db_id>/` (requires
    /// BepInEx 6.x IL2CPP ARM64 to be installed).
    fn deploy_native(
        &self,
        detected: &DetectedGame,
        db: &Arc<ModDatabase>,
    ) -> std::result::Result<DeployResult, DeployerError> {
        let mods_dir = resolve_mods_dir();
        let install_dir = resolve_game_install_dir(detected);
        deploy_native_inner(detected, db, &mods_dir, install_dir.as_deref())
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register() {
    crate::games::register_plugin(Arc::new(ParalivesNativePlugin));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::with_plugin;
    use crate::runtime::{Architecture, NativeSource};

    // ── Test infrastructure ─────────────────────────────────────────────────

    /// Build a synthetic `NativeAppCandidate` for use in unit tests.
    ///
    /// Only `bundle_identifier`, `bundle_executable`, `sandboxed`, `source`,
    /// and `architecture` are read by the detection logic — other InfoPlist
    /// fields are left as `None`.
    fn fake_candidate(
        bundle_path: &str,
        bundle_id: &str,
        exe_name: &str,
        sandboxed: bool,
        source: NativeSource,
        arch: Architecture,
    ) -> crate::native_scanner::NativeAppCandidate {
        crate::native_scanner::NativeAppCandidate {
            bundle_path: std::path::PathBuf::from(bundle_path),
            info: crate::plist::InfoPlist {
                bundle_identifier: bundle_id.to_string(),
                bundle_executable: exe_name.to_string(),
                short_version: None,
                category: None,
            },
            architecture: arch,
            source,
            sandboxed,
        }
    }

    /// Build a synthetic `DetectedGame` with a native runtime for Paralives.
    fn fake_detected_native(bundle_path: &Path) -> DetectedGame {
        let game_path = bundle_path.join("Contents").join("MacOS");
        DetectedGame {
            game_id: "paralives_native".into(),
            display_name: "Paralives".into(),
            nexus_slug: "paralives".into(),
            game_path: game_path.clone(),
            exe_path: Some(game_path.join("Paralives")),
            data_dir: resolve_mods_dir(),
            runtime: crate::runtime::GameRuntime::Native(crate::runtime::NativeContext {
                app_bundle_path: bundle_path.to_path_buf(),
                game_data_root: game_path,
                architecture: Architecture::AppleSilicon,
                sandboxed: false,
                source: NativeSource::Steam,
            }),
            steam_app_id: Some(PARALIVES_STEAM_APP_ID.to_string()),
        }
    }

    /// Write a minimal arm64 single-arch Mach-O header to `path`.
    /// 32 bytes: magic (LE 0xFEEDFACF) + cputype (LE arm64 = 0x0100000C) + padding.
    fn write_arm64_macho(path: &Path) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0xFEED_FACFu32.to_le_bytes()); // MH_MAGIC_64
        bytes.extend_from_slice(&0x0100_000Cu32.to_le_bytes()); // cputype = arm64
        bytes.extend(std::iter::repeat(0u8).take(28));
        std::fs::write(path, &bytes).expect("write arm64 macho");
    }

    /// Write a minimal x86_64 single-arch Mach-O header to `path`.
    fn write_x86_64_macho(path: &Path) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0xFEED_FACFu32.to_le_bytes()); // MH_MAGIC_64
        bytes.extend_from_slice(&0x0100_0007u32.to_le_bytes()); // cputype = x86_64
        bytes.extend(std::iter::repeat(0u8).take(28));
        std::fs::write(path, &bytes).expect("write x86_64 macho");
    }

    /// Synthesize a full BepInEx install layout under `dir`:
    /// - `BepInEx/core/BepInEx.Core.dll`
    /// - `libdoorstop.dylib` (arm64 when `arm64 == true`, else x86_64)
    /// - `changelog.txt` (optional)
    fn make_bepinex_layout(dir: &Path, arm64: bool, with_changelog: bool) {
        let core_dir = dir.join("BepInEx").join("core");
        std::fs::create_dir_all(&core_dir).expect("create core dir");
        std::fs::write(core_dir.join("BepInEx.Core.dll"), b"fake dll")
            .expect("write core dll");

        let dylib_path = dir.join("libdoorstop.dylib");
        if arm64 {
            write_arm64_macho(&dylib_path);
        } else {
            write_x86_64_macho(&dylib_path);
        }

        if with_changelog {
            std::fs::write(
                dir.join("changelog.txt"),
                "# 6.0.0-pre.2\nSome changelog text\n",
            )
            .expect("write changelog");
        }
    }

    // ── 1. Plugin metadata ──────────────────────────────────────────────────

    #[test]
    fn paralives_plugin_metadata() {
        let plugin = ParalivesNativePlugin;
        assert_eq!(plugin.game_id(), "paralives_native");
        assert_eq!(plugin.display_name(), "Paralives (Native)");
        assert_eq!(plugin.nexus_slug(), "paralives");
    }

    // ── 2. Detection: filter by bundle identifier ───────────────────────────

    #[test]
    fn paralives_detect_native_filters_by_bundle_id() {
        let candidates = vec![
            fake_candidate(
                "/Users/user/Library/Application Support/Steam/steamapps/common/Paralives/Paralives.app",
                PARALIVES_BUNDLE_IDENTIFIER,
                "Paralives",
                false,
                NativeSource::Steam,
                Architecture::AppleSilicon,
            ),
            fake_candidate(
                "/Applications/OtherGame.app",
                "com.other.game",
                "OtherGame",
                false,
                NativeSource::SystemApplications,
                Architecture::Universal,
            ),
        ];

        let results = detect_from_candidates(candidates);
        assert_eq!(results.len(), 1, "only the Paralives bundle should match");
        assert_eq!(results[0].game_id, "paralives_native");
        assert_eq!(results[0].display_name, "Paralives");
    }

    // ── 3. Detection: executable-name fallback ──────────────────────────────

    #[test]
    fn paralives_detect_native_falls_back_to_executable_name() {
        let candidates = vec![fake_candidate(
            "/Users/user/Games/Paralives/Paralives.app",
            "com.unknown.paralives", // non-standard bundle id
            "Paralives",             // matching executable name
            false,
            NativeSource::Manual,
            Architecture::AppleSilicon,
        )];

        let results = detect_from_candidates(candidates);
        assert_eq!(
            results.len(),
            1,
            "executable-name fallback should match non-standard bundle"
        );
        assert_eq!(results[0].game_id, "paralives_native");
    }

    // ── 4. Detection: sandboxed candidates skipped ─────────────────────────

    #[test]
    fn paralives_detect_native_skips_sandboxed() {
        let candidates = vec![
            fake_candidate(
                "/Applications/Paralives.app",
                PARALIVES_BUNDLE_IDENTIFIER,
                "Paralives",
                true, // sandboxed
                NativeSource::SystemApplications,
                Architecture::AppleSilicon,
            ),
            fake_candidate(
                "/Users/user/Games/Paralives.app",
                PARALIVES_BUNDLE_IDENTIFIER,
                "Paralives",
                false, // not sandboxed
                NativeSource::Steam,
                Architecture::AppleSilicon,
            ),
        ];

        let results = detect_from_candidates(candidates);
        assert_eq!(results.len(), 1, "sandboxed candidate must be skipped");
        let ctx = match &results[0].runtime {
            crate::runtime::GameRuntime::Native(n) => n,
            _ => panic!("expected Native runtime"),
        };
        assert!(!ctx.sandboxed);
    }

    // ── 5. Detection: Steam source populates steam_app_id ──────────────────

    #[test]
    fn paralives_detect_native_populates_steam_app_id_for_steam_source() {
        let candidates = vec![fake_candidate(
            "/Users/user/Library/Application Support/Steam/steamapps/common/Paralives/Paralives.app",
            PARALIVES_BUNDLE_IDENTIFIER,
            "Paralives",
            false,
            NativeSource::Steam,
            Architecture::AppleSilicon,
        )];

        let results = detect_from_candidates(candidates);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].steam_app_id,
            Some(PARALIVES_STEAM_APP_ID.to_string()),
            "Steam source must set steam_app_id"
        );
    }

    // ── 6. Detection: non-Steam sources omit steam_app_id ──────────────────

    #[test]
    fn paralives_detect_native_omits_steam_app_id_for_other_sources() {
        let non_steam_sources = [
            NativeSource::Gog,
            NativeSource::Manual,
            NativeSource::SystemApplications,
        ];

        for source in non_steam_sources {
            let candidates = vec![fake_candidate(
                "/Applications/Paralives.app",
                PARALIVES_BUNDLE_IDENTIFIER,
                "Paralives",
                false,
                source,
                Architecture::AppleSilicon,
            )];
            let results = detect_from_candidates(candidates);
            assert_eq!(results.len(), 1);
            assert!(
                results[0].steam_app_id.is_none(),
                "source {:?} must not set steam_app_id",
                source
            );
        }
    }

    // ── 7. resolve_mods_dir returns persistent data path ───────────────────

    #[test]
    fn paralives_resolve_mods_dir_returns_persistent_data_path() {
        let mods_dir = resolve_mods_dir();
        let home = dirs::home_dir().expect("home dir must be available in test environment");
        let expected = home.join("Library/Application Support/com.Paralives.Paralives/Mods");
        assert_eq!(
            mods_dir, expected,
            "mods dir must be the Unity persistent data path"
        );
    }

    // ── 8. Artifact rejection: Windows .exe ─────────────────────────────────

    #[test]
    fn rejects_paralives_artifact_rejects_windows_exe() {
        assert!(
            rejects_paralives_artifact("CheatMod.exe"),
            ".exe must be rejected"
        );
        assert!(
            rejects_paralives_artifact("subdir/launcher.exe"),
            ".exe in subdirectory must be rejected"
        );
    }

    // ── 9. Artifact rejection: winhttp.dll and doorstop_config.ini ─────────
    // NOTE: plain .dll is now a BepInEx plugin candidate, NOT rejected here.

    #[test]
    fn rejects_paralives_artifact_still_refuses_exe_and_winhttp() {
        // .exe: always refused.
        assert!(
            rejects_paralives_artifact("Launcher.exe"),
            ".exe must still be rejected"
        );
        assert!(
            rejects_paralives_artifact("subdir/tool.exe"),
            ".exe in subdir must still be rejected"
        );

        // winhttp.dll: Windows-flavor BepInEx Doorstop — refused.
        assert!(
            rejects_paralives_artifact("winhttp.dll"),
            "winhttp.dll must be rejected"
        );
        assert!(
            rejects_paralives_artifact("WINHTTP.DLL"),
            "winhttp.dll (uppercase) must be rejected"
        );

        // doorstop_config.ini: belongs to BepInEx install, not a mod.
        assert!(
            rejects_paralives_artifact("doorstop_config.ini"),
            "doorstop_config.ini must be rejected"
        );

        // Regression: plain .dll is now legal (routes to BepInEx bucket).
        assert!(
            !rejects_paralives_artifact("SomeMod.dll"),
            ".dll must NOT be rejected by rejects_paralives_artifact (it's a BepInEx candidate)"
        );
    }

    // ── 10. Artifact rejection: winhttp.dll loader ──────────────────────────

    #[test]
    fn rejects_paralives_artifact_rejects_winhttp_loader() {
        assert!(
            rejects_paralives_artifact("winhttp.dll"),
            "winhttp.dll must be rejected"
        );
        assert!(
            rejects_paralives_artifact("WINHTTP.DLL"),
            "winhttp.dll (uppercase) must be rejected"
        );
        assert!(
            rejects_paralives_artifact("doorstop_config.ini"),
            "doorstop_config.ini must be rejected"
        );
    }

    // ── 11. Artifact acceptance: official data-mod formats ──────────────────

    #[test]
    fn rejects_paralives_artifact_accepts_png_fbx_json_etc() {
        let accepted = [
            "textures/skin_01.png",
            "models/chair.fbx",
            "models/table.obj",
            "ui/button.jpg",
            "ui/icon.jpeg",
            "animations/walk.catalog",
            "audio/ambient.ogg",
            "audio/click.wav",
            "config/traits.json",
            "fonts/custom.ttf",
        ];
        for path in &accepted {
            assert!(
                !rejects_paralives_artifact(path),
                "'{}' should be accepted as a valid Paralives data-mod format",
                path
            );
        }
    }

    // ── 12. classify_files: top-level .dll → BepInEx ───────────────────────

    #[test]
    fn classify_files_routes_top_level_dll_to_bepinex() {
        let staged = vec![
            (PathBuf::from("/staging/MyPlugin.dll"), "MyPlugin.dll".to_string()),
            (PathBuf::from("/staging/config.json"), "config.json".to_string()),
        ];
        let classified = classify_files(&staged);
        assert_eq!(
            classified.bepinex_plugin_files.len(),
            1,
            "top-level .dll must go to BepInEx bucket"
        );
        assert_eq!(
            classified.bepinex_plugin_files[0].1,
            "MyPlugin.dll"
        );
        assert_eq!(
            classified.data_mod_files.len(),
            1,
            "config.json must go to data bucket"
        );
    }

    // ── 13. classify_files: BepInEx/plugins/ path → BepInEx ────────────────

    #[test]
    fn classify_files_routes_bepinex_plugins_path_to_bepinex() {
        let staged = vec![
            (
                PathBuf::from("/staging/BepInEx/plugins/MyMod.dll"),
                "BepInEx/plugins/MyMod.dll".to_string(),
            ),
            (
                PathBuf::from("/staging/BepInEx/plugins/SubDir/Helper.dll"),
                "BepInEx/plugins/SubDir/Helper.dll".to_string(),
            ),
        ];
        let classified = classify_files(&staged);
        assert_eq!(
            classified.bepinex_plugin_files.len(),
            2,
            "both BepInEx/plugins/ paths must be in the BepInEx bucket"
        );
        assert!(
            classified.data_mod_files.is_empty(),
            "data bucket must be empty"
        );
    }

    // ── 14. classify_files: data formats → data bucket ──────────────────────

    #[test]
    fn classify_files_routes_data_formats_to_data() {
        let staged = vec![
            (PathBuf::from("/staging/model.fbx"), "model.fbx".to_string()),
            (PathBuf::from("/staging/tex/skin.png"), "tex/skin.png".to_string()),
            (PathBuf::from("/staging/sfx/click.ogg"), "sfx/click.ogg".to_string()),
            (PathBuf::from("/staging/meta.json"), "meta.json".to_string()),
        ];
        let classified = classify_files(&staged);
        assert!(
            classified.bepinex_plugin_files.is_empty(),
            "no BepInEx files in a pure data-mod archive"
        );
        assert_eq!(classified.data_mod_files.len(), 4);
    }

    // ── 15. classify_files: mixed archive ───────────────────────────────────

    #[test]
    fn classify_files_handles_mixed_archive() {
        let staged = vec![
            (
                PathBuf::from("/staging/BepInEx/plugins/CoreMod.dll"),
                "BepInEx/plugins/CoreMod.dll".to_string(),
            ),
            (
                PathBuf::from("/staging/assets/icon.png"),
                "assets/icon.png".to_string(),
            ),
            (
                PathBuf::from("/staging/Standalone.dll"),
                "Standalone.dll".to_string(),
            ),
            (
                PathBuf::from("/staging/config/settings.json"),
                "config/settings.json".to_string(),
            ),
        ];
        let classified = classify_files(&staged);
        // CoreMod.dll (BepInEx path) + Standalone.dll (top-level) → BepInEx bucket
        assert_eq!(
            classified.bepinex_plugin_files.len(),
            2,
            "BepInEx path DLL + top-level DLL must both be in BepInEx bucket"
        );
        // icon.png + settings.json → data bucket
        assert_eq!(
            classified.data_mod_files.len(),
            2,
            "data assets must be in data bucket"
        );
    }

    // ── deploy_native_inner: sandbox refusal ────────────────────────────────

    #[test]
    fn paralives_deploy_native_refuses_sandboxed_game() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());

        let detected = DetectedGame {
            game_id: "paralives_native".into(),
            display_name: "Paralives".into(),
            nexus_slug: "paralives".into(),
            game_path: tmp.path().to_path_buf(),
            exe_path: None,
            data_dir: resolve_mods_dir(),
            runtime: crate::runtime::GameRuntime::Native(crate::runtime::NativeContext {
                app_bundle_path: tmp.path().to_path_buf(),
                game_data_root: tmp.path().to_path_buf(),
                architecture: Architecture::AppleSilicon,
                sandboxed: true, // sandboxed
                source: NativeSource::Steam,
            }),
            steam_app_id: Some(PARALIVES_STEAM_APP_ID.to_string()),
        };

        let result = deploy_native_inner(&detected, &db, &mods_dir, None);
        assert!(result.is_err(), "must refuse sandboxed game");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("sandboxed"),
            "error must mention 'sandboxed': {msg}"
        );
        assert!(
            !mods_dir.exists(),
            "mods_dir must not be created for sandboxed game"
        );
    }

    // ── deploy_native_inner: snapshot created ───────────────────────────────

    #[test]
    fn paralives_deploy_native_creates_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        crate::rollback::init_schema(&db).unwrap();

        // Add a staging mod with a simple data file.
        let staging_dir = tmp.path().join("staging");
        std::fs::create_dir_all(&staging_dir).unwrap();
        std::fs::write(staging_dir.join("mod_data.json"), b"{}").unwrap();

        let mod_id = db
            .add_mod(
                "paralives_native",
                "",
                None,
                "Test Mod",
                "1.0.0",
                "test_mod.zip",
                &[],
            )
            .unwrap();
        db.set_staging_path(mod_id, &staging_dir.to_string_lossy())
            .unwrap();

        let detected = fake_detected_native(tmp.path());
        deploy_native_inner(&detected, &db, &mods_dir, None).unwrap();

        let snapshots =
            crate::rollback::list_snapshots(&db, "paralives_native", "").unwrap();
        assert!(
            !snapshots.is_empty(),
            "deploy_native_inner should create a snapshot"
        );
        assert_eq!(
            snapshots[0].name, "paralives-deploy",
            "snapshot name must be 'paralives-deploy'"
        );
    }

    // ── deploy_native_inner: happy path (data mods) ─────────────────────────

    #[test]
    fn paralives_deploy_native_copies_files_to_mods_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());

        let staging_dir = tmp.path().join("staging");
        std::fs::create_dir_all(staging_dir.join("textures")).unwrap();
        std::fs::write(staging_dir.join("config.json"), b"{}").unwrap();
        std::fs::write(staging_dir.join("textures/skin.png"), b"PNG").unwrap();

        let mod_id = db
            .add_mod(
                "paralives_native",
                "",
                None,
                "Asset Mod",
                "1.0.0",
                "asset_mod.zip",
                &[],
            )
            .unwrap();
        db.set_staging_path(mod_id, &staging_dir.to_string_lossy())
            .unwrap();

        let detected = fake_detected_native(tmp.path());
        let result = deploy_native_inner(&detected, &db, &mods_dir, None).unwrap();

        assert_eq!(result.deployed_count, 2, "config.json + textures/skin.png");
        assert!(mods_dir.join("config.json").exists());
        assert!(mods_dir.join("textures/skin.png").exists());
    }

    // ── deploy_native_inner: BepInEx not installed ──────────────────────────

    #[test]
    fn deploy_native_refuses_bepinex_plugin_when_bepinex_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        // game_install_dir is a vanilla dir (no BepInEx).
        let install_dir = tmp.path().join("GameInstall");
        std::fs::create_dir_all(&install_dir).unwrap();

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());

        // Stage a mod with a BepInEx plugin DLL under BepInEx/plugins/.
        let staging_dir = tmp.path().join("staging");
        std::fs::create_dir_all(staging_dir.join("BepInEx/plugins")).unwrap();
        std::fs::write(
            staging_dir.join("BepInEx/plugins/MyMod.dll"),
            b"MZ fake dll",
        )
        .unwrap();

        let mod_id = db
            .add_mod(
                "paralives_native",
                "",
                None,
                "Script Mod",
                "1.0.0",
                "script_mod.zip",
                &[],
            )
            .unwrap();
        db.set_staging_path(mod_id, &staging_dir.to_string_lossy())
            .unwrap();

        // Build a bundle path whose parent is install_dir.
        let bundle_path = install_dir.join("Paralives.app");
        std::fs::create_dir_all(&bundle_path).unwrap();
        let detected = fake_detected_native(&bundle_path);

        let result =
            deploy_native_inner(&detected, &db, &mods_dir, Some(install_dir.as_path()));
        assert!(result.is_err(), "must refuse when BepInEx is not installed");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("BepInEx is required"),
            "error must say 'BepInEx is required': {msg}"
        );
    }

    // ── deploy_native_inner: BepInEx installed but wrong arch ───────────────

    #[test]
    fn deploy_native_refuses_bepinex_plugin_when_mac_unsupported() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        let install_dir = tmp.path().join("GameInstall");
        std::fs::create_dir_all(&install_dir).unwrap();

        // Install BepInEx with an x86_64-only dylib (BepInEx 5.x style).
        make_bepinex_layout(&install_dir, false /* x86_64 */, false);

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());

        let staging_dir = tmp.path().join("staging");
        std::fs::create_dir_all(staging_dir.join("BepInEx/plugins")).unwrap();
        std::fs::write(
            staging_dir.join("BepInEx/plugins/MyMod.dll"),
            b"MZ fake dll",
        )
        .unwrap();

        let mod_id = db
            .add_mod(
                "paralives_native",
                "",
                None,
                "Script Mod",
                "1.0.0",
                "script_mod.zip",
                &[],
            )
            .unwrap();
        db.set_staging_path(mod_id, &staging_dir.to_string_lossy())
            .unwrap();

        let bundle_path = install_dir.join("Paralives.app");
        std::fs::create_dir_all(&bundle_path).unwrap();
        let detected = fake_detected_native(&bundle_path);

        let result =
            deploy_native_inner(&detected, &db, &mods_dir, Some(install_dir.as_path()));
        assert!(result.is_err(), "must refuse x86_64-only BepInEx");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("wrong architecture"),
            "error must mention 'wrong architecture': {msg}"
        );
    }

    // ── deploy_native_inner: BepInEx installed arm64, plugin deployed ────────

    #[test]
    fn deploy_native_copies_bepinex_plugin_to_plugins_dir_when_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        let install_dir = tmp.path().join("GameInstall");
        std::fs::create_dir_all(&install_dir).unwrap();

        // Install BepInEx with an arm64 dylib (BepInEx 6.x style).
        make_bepinex_layout(&install_dir, true /* arm64 */, true);

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());

        let staging_dir = tmp.path().join("staging");
        std::fs::create_dir_all(staging_dir.join("BepInEx/plugins")).unwrap();
        std::fs::write(
            staging_dir.join("BepInEx/plugins/MyMod.dll"),
            b"MZ fake dll",
        )
        .unwrap();

        let mod_db_id = db
            .add_mod(
                "paralives_native",
                "",
                None,
                "Script Mod",
                "1.0.0",
                "script_mod.zip",
                &[],
            )
            .unwrap();
        db.set_staging_path(mod_db_id, &staging_dir.to_string_lossy())
            .unwrap();

        let bundle_path = install_dir.join("Paralives.app");
        std::fs::create_dir_all(&bundle_path).unwrap();
        let detected = fake_detected_native(&bundle_path);

        let result =
            deploy_native_inner(&detected, &db, &mods_dir, Some(install_dir.as_path()))
                .expect("deploy must succeed with arm64 BepInEx");

        assert_eq!(result.deployed_count, 1, "one plugin file deployed");

        // Plugin must be in <install_dir>/BepInEx/plugins/<mod_db_id>/MyMod.dll
        let expected_path = install_dir
            .join("BepInEx")
            .join("plugins")
            .join(mod_db_id.to_string())
            .join("MyMod.dll");
        assert!(
            expected_path.exists(),
            "plugin must be deployed at {}: file not found",
            expected_path.display()
        );

        // Data mods dir must not contain the DLL.
        assert!(
            !mods_dir.join("BepInEx/plugins/MyMod.dll").exists(),
            "DLL must not appear in the data Mods/ dir"
        );
    }

    // ── deploy_native_inner: legacy test renamed/updated ────────────────────
    // The old `paralives_deploy_native_refuses_bepinex_dll` tested that a
    // BepInEx/plugins/ DLL caused a hard rejection. Now DLLs under BepInEx/
    // are routed to the BepInEx bucket. The test is superseded by:
    //   - deploy_native_refuses_bepinex_plugin_when_bepinex_not_installed
    //   - deploy_native_copies_bepinex_plugin_to_plugins_dir_when_installed
    // We keep a renamed version that validates the new routing (no longer
    // returns an error when BepInEx is absent is checked elsewhere, but here
    // we confirm the file is NOT sent to data Mods/).

    #[test]
    fn paralives_deploy_native_bepinex_dll_routes_to_bepinex_bucket_not_data() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        let install_dir = tmp.path().join("GameInstall");
        std::fs::create_dir_all(&install_dir).unwrap();

        // BepInEx arm64 installed.
        make_bepinex_layout(&install_dir, true, false);

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());

        let staging_dir = tmp.path().join("staging");
        std::fs::create_dir_all(staging_dir.join("BepInEx/plugins")).unwrap();
        std::fs::write(
            staging_dir.join("BepInEx/plugins/ScriptMod.dll"),
            b"MZ fake dll",
        )
        .unwrap();

        let mod_id = db
            .add_mod(
                "paralives_native",
                "",
                None,
                "Script Mod",
                "1.0.0",
                "script_mod.zip",
                &[],
            )
            .unwrap();
        db.set_staging_path(mod_id, &staging_dir.to_string_lossy())
            .unwrap();

        let bundle_path = install_dir.join("Paralives.app");
        std::fs::create_dir_all(&bundle_path).unwrap();
        let detected = fake_detected_native(&bundle_path);

        let result =
            deploy_native_inner(&detected, &db, &mods_dir, Some(install_dir.as_path()))
                .expect("deploy must succeed");

        // DLL must NOT appear in the data Mods/ directory.
        assert!(
            !mods_dir.exists() || !mods_dir.join("BepInEx").exists(),
            "DLL must not be deployed to data Mods/ dir"
        );
        // Must be in the BepInEx plugins dir.
        let plugin_dir = install_dir
            .join("BepInEx")
            .join("plugins")
            .join(mod_id.to_string());
        assert!(
            plugin_dir.join("ScriptMod.dll").exists(),
            "DLL must be in BepInEx plugins dir"
        );
        assert_eq!(result.deployed_count, 1);
    }

    // ── Plugin registration ─────────────────────────────────────────────────

    #[test]
    fn paralives_native_plugin_registers() {
        crate::games::register_plugin(std::sync::Arc::new(ParalivesNativePlugin));
        let result = with_plugin("paralives_native", |p| p.display_name().to_owned());
        assert_eq!(result, Some("Paralives (Native)".to_owned()));
    }
}
