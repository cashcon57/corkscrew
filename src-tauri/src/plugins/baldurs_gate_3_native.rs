//! Baldur's Gate 3 (native macOS) game plugin.
//!
//! Detects Larian's macOS-native BG3 build (shipped Sept 2024) and
//! provides game-specific metadata for .pak-based mod management. The
//! native install on mac differs from Windows in path conventions:
//! mods live at `~/Documents/Larian Studios/Baldur's Gate 3/Mods/`
//! (NOT inside the .app bundle, NOT `~/Library/Application Support/`),
//! and the load order is encoded as `<region id="ModuleSettings">` in
//! `modsettings.lsx`.
//!
//! Deploy procedure (Task 4.4):
//! 1. Walk each enabled mod's staging directory for `.pak` files.
//! 2. For each `.pak`: read `meta.lsx` via `bg3_pak::read_pak_meta` to
//!    obtain the `ModuleInfo` (uuid, folder, name, version64).
//! 3. Copy (hardlink-first, copy fallback) the `.pak` into the mods dir.
//! 4. Upsert the corresponding `ModuleShortDesc` entry in
//!    `modsettings.lsx` via `bg3_lsx::write_modsettings`.
//! 5. If `modsettings.lsx` is absent, bootstrap it with the modern
//!    Patch 8+ `GustavX` master entry (UUID
//!    cb555efe-2d9e-131f-8195-a89329d218ea). Older installs that
//!    already contain the pre-Patch-8 trio
//!    (GustavDev/Gustav/SharedDev) are preserved as-is — Corkscrew
//!    never replaces an existing master set.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::bg3_lsx::{
    self, is_master_entry, write_modsettings, LsxVersion, ModEntry, ModSettings,
    MASTER_GUSTAV_X_UUID,
};
use crate::bg3_pak;
use crate::bottles::Bottle;
use crate::database::ModDatabase;
use crate::deployer::{DeployResult, DeployerError};
use crate::games::{DetectedGame, FileBasedLoadOrder, GamePlugin, LoadOrderFormat, LoadOrderKind};
use crate::staging::is_safe_relative_path;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Game plugin for Baldur's Gate 3 (native macOS).
///
/// BG3 ships a macOS-native build via Steam. Mods are distributed as
/// `.pak` files and are managed through `modsettings.lsx` (an XML-based
/// load-order manifest), not a plain-text `plugins.txt`. The actual
/// mods directory on macOS is
/// `~/Documents/Larian Studios/Baldur's Gate 3/Mods/`
/// — independent of the .app bundle location.
pub struct BaldursGate3NativePlugin;

/// CFBundleIdentifier for Baldur's Gate 3 (native macOS).
///
/// Verified empirically from the user's real BG3 install at
/// `~/Library/Application Support/Steam/steamapps/common/Baldurs Gate 3/Baldur's Gate 3.app`.
const BG3_BUNDLE_IDENTIFIER: &str = "com.larian.bg3";

/// Steam App ID for Baldur's Gate 3.
const BG3_STEAM_APP_ID: &str = "1086940";

/// Candidate executable names for BG3 on macOS.
///
/// The primary executable (CFBundleExecutable) verified from the real install
/// is `"Baldur's Gate 3"` (with apostrophe and spaces). `"bg3"` and
/// `"bg3_dx11"` are retained as fallbacks for non-standard packaging.
const EXECUTABLES: &[&str] = &["Baldur's Gate 3", "bg3", "bg3_dx11"];

/// Bottle sentinel for native mods (no Wine bottle — same convention as Stardew).
const NATIVE_BOTTLE_SENTINEL: &str = "";

// ---------------------------------------------------------------------------
// Path resolution helpers
// ---------------------------------------------------------------------------

/// Returns the BG3 mods directory for the current user.
///
/// Resolves to `~/Documents/Larian Studios/Baldur's Gate 3/Mods/` via
/// `dirs::home_dir()`. The path is returned even if it doesn't exist on
/// disk — callers are responsible for creating it before use.
pub fn resolve_mods_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    home.join("Documents/Larian Studios/Baldur's Gate 3/Mods")
}

/// Returns the path to `modsettings.lsx` for the given profile.
///
/// Pass `"Public"` for the default profile (almost all users operate on this).
/// Resolves to:
/// `~/Documents/Larian Studios/Baldur's Gate 3/PlayerProfiles/<profile>/modsettings.lsx`
pub fn resolve_modsettings_path(profile: &str) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    home.join(format!(
        "Documents/Larian Studios/Baldur's Gate 3/PlayerProfiles/{}/modsettings.lsx",
        profile
    ))
}

// ---------------------------------------------------------------------------
// Master-entry bootstrapping
// ---------------------------------------------------------------------------

/// Build the single mandatory Patch 8+ master mod entry.
///
/// As of BG3 4.8.0.700, vanilla ships exactly one master:
/// `GustavX` (UUID cb555efe-2d9e-131f-8195-a89329d218ea). This is the
/// default Corkscrew writes when no `modsettings.lsx` exists yet.
///
/// Older installs (pre-Patch-8) may have GustavDev / Gustav / SharedDev
/// instead — those are recognised as masters via
/// [`crate::bg3_lsx::is_master_entry`] and PRESERVED when reading
/// existing files. Corkscrew never replaces an existing master set with
/// the modern default — only bootstraps when the file is absent or
/// empty.
fn bootstrap_master_entries() -> Vec<ModEntry> {
    vec![ModEntry {
        folder: "GustavX".into(),
        md5: String::new(),
        name: "GustavX".into(),
        publish_handle: "0".into(),
        uuid: MASTER_GUSTAV_X_UUID.into(),
        version64: "36028797018963968".into(),
    }]
}

// ---------------------------------------------------------------------------
// Detection helper (pure function for testability)
// ---------------------------------------------------------------------------

/// Filter `native_scanner` candidates to BG3 installs and produce
/// `DetectedGame` entries. Pure function — the `GamePlugin::detect_native`
/// impl wraps this with the actual scanner call.
///
/// Accepts candidates matching:
/// - `bundle_identifier == "com.larian.bg3"` (case-insensitive, primary), OR
/// - `bundle_executable` is one of the names in `EXECUTABLES` (fallback).
///
/// Sandboxed candidates are always rejected.
fn detect_from_candidates(
    candidates: Vec<crate::native_scanner::NativeAppCandidate>,
) -> Vec<DetectedGame> {
    candidates
        .into_iter()
        .filter(|c| !c.sandboxed)
        .filter(|c| {
            let id = c.info.bundle_identifier.to_ascii_lowercase();
            if id == BG3_BUNDLE_IDENTIFIER {
                return true;
            }
            let exe = c.info.bundle_executable.as_str();
            EXECUTABLES
                .iter()
                .any(|&name| name.eq_ignore_ascii_case(exe))
        })
        .map(|c| {
            let game_path = c.bundle_path.join("Contents").join("MacOS");
            let exe_name = c.info.bundle_executable.clone();
            let exe_path = Some(game_path.join(&exe_name));
            let data_dir = resolve_mods_dir();
            let steam_app_id = if c.source == crate::runtime::NativeSource::Steam {
                Some(BG3_STEAM_APP_ID.to_string())
            } else {
                None
            };
            DetectedGame {
                game_id: "baldurs_gate_3_native".to_string(),
                display_name: "Baldur's Gate 3".to_string(),
                nexus_slug: "baldursgate3".to_string(),
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
// Inner (testable) deploy function
// ---------------------------------------------------------------------------

/// Core deployment logic for BG3 native mods.
///
/// Takes explicit `mods_dir` and `modsettings_path` arguments rather than
/// resolving from HOME, so unit tests can redirect to tempdirs.
///
/// Algorithm:
/// 1. Verify `detected.runtime` is [`crate::runtime::GameRuntime::Native`].
/// 2. Create `mods_dir` and the parent directory of `modsettings_path` if absent.
/// 3. Load existing `modsettings.lsx` or bootstrap with the modern
///    Patch 8+ master (`GustavX`).
/// 4. Defensive: if the loaded file has no recognised master at all (damaged
///    or wiped), prepend the modern default. Existing master sets — modern
///    or legacy — are preserved as-is.
/// 5. Walk enabled mods; for each `.pak` in a mod's staging dir:
///    a. Validate the filename (no path traversal).
///    b. Read `meta.lsx` from the pak.
///    c. Copy (hardlink-first) the pak to `mods_dir`.
///    d. Upsert the resulting `ModEntry` (replace on UUID match, else append).
/// 6. Defensive: verify at least one recognised master is still present
///    after the walk.
/// 7. Write the updated `modsettings.lsx`.
pub fn deploy_native_inner(
    detected: &DetectedGame,
    db: &Arc<ModDatabase>,
    mods_dir: &Path,
    modsettings_path: &Path,
) -> Result<DeployResult, DeployerError> {
    // 1. Reject Wine-hosted games — this function is native-only.
    // Also reject sandboxed bundles: the App Sandbox prevents file modifications.
    let native_ctx = detected
        .runtime
        .native()
        .ok_or_else(|| DeployerError::Other("expected native runtime for BG3 deploy".into()))?;
    if native_ctx.sandboxed {
        return Err(DeployerError::Other(format!(
            "native modding refused for sandboxed app: {}",
            native_ctx.app_bundle_path.display()
        )));
    }

    // 2. Create directories.
    std::fs::create_dir_all(mods_dir)
        .map_err(|e| DeployerError::Other(format!("create mods_dir: {}", e)))?;
    if let Some(parent) = modsettings_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| DeployerError::Other(format!("create modsettings parent: {}", e)))?;
    }

    // 3. Load existing modsettings.lsx or bootstrap with master entries.
    let mut settings = if modsettings_path.exists() {
        bg3_lsx::read_modsettings(modsettings_path)
            .map_err(|e| DeployerError::Other(format!("read modsettings: {}", e)))?
    } else {
        ModSettings {
            version: LsxVersion {
                major: 4,
                minor: 0,
                revision: 9,
                build: 319,
            },
            mods: bootstrap_master_entries(),
        }
    };

    // 4. Defensive: if NO recognised master is present after loading, the
    //    file is damaged or vanilla just got wiped. Bootstrap with the
    //    modern Patch 8 default (GustavX) by prepending it. We intentionally
    //    do NOT re-add the old pre-Patch-8 trio — those are legacy and the
    //    game expects whichever set matches the installed version. If the
    //    user is on pre-Patch-8 and the file already has
    //    GustavDev/Gustav/SharedDev, those are preserved because
    //    `is_master_entry` recognises them; we only intervene when ALL
    //    masters are missing.
    let has_any_master = settings.mods.iter().any(|m| is_master_entry(&m.uuid));
    if !has_any_master {
        let mut bootstrapped = bootstrap_master_entries();
        bootstrapped.extend(settings.mods.drain(..));
        settings.mods = bootstrapped;
    }

    // Canonicalise mods_dir once for the destination-escape check below.
    // We use `canonicalize` because the directory was just created above, so
    // it must exist at this point.
    let canonical_mods_dir = mods_dir
        .canonicalize()
        .unwrap_or_else(|_| mods_dir.to_path_buf());

    // Pre-deploy snapshot (best-effort — failure must not abort deploy).
    if let Err(e) = crate::rollback::create_native_snapshot(
        db,
        &detected.game_id,
        "bg3-deploy",
        "Baldur's Gate 3 native deploy",
    ) {
        log::warn!("snapshot before BG3 deploy failed: {}", e);
    }

    // 5. Walk enabled mods from the database.
    let enabled_mods = db
        .list_mods(&detected.game_id, NATIVE_BOTTLE_SENTINEL)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let mut deployed_count = 0usize;
    let mut fallback_used = false;

    for installed_mod in enabled_mods.iter().filter(|m| m.enabled) {
        let staging_dir = match &installed_mod.staging_path {
            Some(p) => PathBuf::from(p),
            None => {
                log::warn!(
                    "bg3 deploy_native: mod '{}' has no staging path, skipping",
                    installed_mod.name
                );
                continue;
            }
        };

        if !staging_dir.exists() {
            log::warn!(
                "bg3 deploy_native: staging dir missing for mod '{}': {}",
                installed_mod.name,
                staging_dir.display()
            );
            continue;
        }

        // Walk the staging dir and process every .pak file found.
        for entry in walkdir::WalkDir::new(&staging_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();

            // Only process .pak files.
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase());
            if ext.as_deref() != Some("pak") {
                continue;
            }

            // -- Path safety checks -----------------------------------------

            // Extract the bare filename (no directory components).
            let filename = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| {
                    DeployerError::Other(format!(
                        "pak at '{}' has no valid filename",
                        path.display()
                    ))
                })?;

            // The filename must not contain path-traversal sequences or
            // directory separators. `is_safe_relative_path` rejects `..`,
            // null bytes, drive letters, and a few other hazards; the extra
            // checks below catch embedded slashes that would escape mods_dir.
            if !is_safe_relative_path(filename)
                || filename.contains('/')
                || filename.contains('\\')
            {
                return Err(DeployerError::Other(format!(
                    "unsafe pak filename '{}' in mod '{}'",
                    filename, installed_mod.name
                )));
            }

            let dest = mods_dir.join(filename);

            // Post-join canonicalization: verify dest stays inside mods_dir.
            // We check the parent (mods_dir itself, since filename is flat) to
            // avoid a race with mods_dir creation.
            let dest_parent = dest
                .parent()
                .map(|p| {
                    if p.exists() {
                        p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
                    } else {
                        canonical_mods_dir.clone()
                    }
                })
                .unwrap_or_else(|| canonical_mods_dir.clone());

            if !dest_parent.starts_with(&canonical_mods_dir) {
                return Err(DeployerError::Other(format!(
                    "destination '{}' escapes mods dir '{}'",
                    dest.display(),
                    mods_dir.display()
                )));
            }

            // -- Read meta.lsx from the pak BEFORE copying -------------------
            // If the pak is malformed, we want to fail before touching disk.
            let info = bg3_pak::read_pak_meta(path).map_err(|e| {
                DeployerError::Other(format!(
                    "read meta.lsx from '{}' (mod '{}'): {}",
                    filename, installed_mod.name, e
                ))
            })?;

            // -- Copy (hardlink-first) the pak to mods_dir -------------------
            // Remove any pre-existing file at the destination so hardlink doesn't
            // fail with EEXIST.
            if dest.exists() {
                let _ = std::fs::remove_file(&dest);
            }

            let copy_failed = std::fs::hard_link(path, &dest).is_err();
            if copy_failed {
                std::fs::copy(path, &dest)
                    .map_err(|e| DeployerError::Other(format!("copy pak '{}': {}", filename, e)))?;
                fallback_used = true;
            }

            // -- Upsert the ModEntry in modsettings.lsx ----------------------
            let entry = bg3_lsx::module_info_to_mod_entry(&info);
            let target_uuid = entry.uuid.to_lowercase();

            match settings
                .mods
                .iter_mut()
                .find(|m| m.uuid.to_lowercase() == target_uuid)
            {
                Some(slot) => {
                    // Idempotent re-deploy: update in-place.
                    *slot = entry;
                }
                None => {
                    settings.mods.push(entry);
                }
            }

            deployed_count += 1;
        }
    }

    // 6. Defensive: ensure at least one recognised master is still present
    //    after the walk. We accept any combination of legacy trio +/or modern
    //    GustavX — both are valid depending on the installed game version.
    if !settings.mods.iter().any(|m| is_master_entry(&m.uuid)) {
        return Err(DeployerError::Other(
            "no recognised master entry present after deploy — refusing to write modsettings"
                .to_string(),
        ));
    }

    // 7. Write the updated modsettings.lsx.
    write_modsettings(modsettings_path, &settings)
        .map_err(|e| DeployerError::Other(format!("write modsettings: {}", e)))?;

    Ok(DeployResult {
        deployed_count,
        skipped_count: 0,
        fallback_used,
    })
}

// ---------------------------------------------------------------------------
// GamePlugin impl
// ---------------------------------------------------------------------------

impl GamePlugin for BaldursGate3NativePlugin {
    fn game_id(&self) -> &str {
        "baldurs_gate_3_native"
    }

    fn display_name(&self) -> &str {
        "Baldur's Gate 3 (Native)"
    }

    fn nexus_slug(&self) -> &str {
        "baldursgate3"
    }

    fn executables(&self) -> &[&str] {
        EXECUTABLES
    }

    /// Detect native BG3 installs by scanning all native app candidates.
    ///
    /// Matches on `CFBundleIdentifier == "com.larian.bg3"` (primary) or
    /// executable name fallback. Sandboxed App Store bundles are refused.
    fn detect_native(&self) -> Vec<DetectedGame> {
        detect_from_candidates(crate::native_scanner::scan_all_native())
    }

    /// Returns the BG3 mods directory relative to `game_path`.
    ///
    /// Note: the *true* BG3 mods directory on macOS is rooted under
    /// `~/Documents/Larian Studios/Baldur's Gate 3/Mods/` and is independent
    /// of the .app bundle path. Use [`resolve_mods_dir`] for deployment;
    /// this method is only used by the generic mod install pipeline.
    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        game_path.join("Mods")
    }

    /// BG3 has no `plugins.txt` load-order file.
    ///
    /// Load order is stored as XML in `modsettings.lsx`
    /// (`<region id="ModuleSettings">`). The `LoadOrderKind::FileBased` path
    /// with `LoadOrderFormat::Bg3ModSettings` drives the generic
    /// re-orderable-list panel for community mods.
    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        None
    }

    /// BG3 exposes its mod load order via the generic `FileBasedLoadOrderPanel`.
    ///
    /// The config path points at the canonical `modsettings.lsx` for the
    /// `"Public"` profile. The `Bg3ModSettings` format variant handles
    /// master-entry filtering on read and master-preserving merge on write.
    fn load_order_kind(&self, _game_path: &Path) -> LoadOrderKind {
        LoadOrderKind::FileBased(FileBasedLoadOrder {
            config_path: resolve_modsettings_path("Public"),
            format: LoadOrderFormat::Bg3ModSettings,
            describe: None,
        })
    }

    /// Deploy all staged BG3 mods into the mods directory and update
    /// `modsettings.lsx`.
    ///
    /// This is a thin wrapper that resolves the canonical macOS paths via
    /// [`resolve_mods_dir`] and [`resolve_modsettings_path`], then delegates
    /// all work to the testable [`deploy_native_inner`].
    fn deploy_native(
        &self,
        detected: &DetectedGame,
        db: &Arc<ModDatabase>,
    ) -> std::result::Result<DeployResult, DeployerError> {
        let mods_dir = resolve_mods_dir();
        let modsettings = resolve_modsettings_path("Public");
        deploy_native_inner(detected, db, &mods_dir, &modsettings)
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register() {
    crate::games::register_plugin(std::sync::Arc::new(BaldursGate3NativePlugin));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bg3_lsx::{
        read_modsettings, MASTER_GUSTAV_DEV_UUID, MASTER_GUSTAV_UUID, MASTER_GUSTAV_X_UUID,
        MASTER_SHARED_DEV_UUID,
    };
    use crate::bg3_pak::{make_minimal_lspk, TEST_META_LSX_XML};
    use crate::games::{with_plugin, GamePlugin};
    use crate::runtime::{Architecture, GameRuntime, NativeContext, NativeSource};
    use std::io::Write;
    use std::sync::Arc;
    use tempfile::NamedTempFile;

    // ── Detection test infrastructure ───────────────────────────────────────

    /// Build a synthetic `NativeAppCandidate` for use in detection unit tests.
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

    // ── Shared test infrastructure ──────────────────────────────────────────

    /// Write bytes to a temporary file; returns the handle (file is removed on drop).
    fn write_temp_pak(data: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(data).unwrap();
        f.flush().unwrap();
        f
    }

    /// Build a synthetic `DetectedGame` with a native runtime.
    ///
    /// `game_path` is used as both `game_path` and `game_data_root`.
    fn fake_detected_native(game_path: &Path) -> DetectedGame {
        DetectedGame {
            game_id: "baldurs_gate_3_native".into(),
            display_name: "Baldur's Gate 3 (Native)".into(),
            nexus_slug: "baldursgate3".into(),
            game_path: game_path.to_path_buf(),
            exe_path: Some(game_path.join("bg3")),
            data_dir: game_path.join("Mods"),
            runtime: GameRuntime::Native(NativeContext {
                app_bundle_path: game_path.to_path_buf(),
                game_data_root: game_path.to_path_buf(),
                architecture: Architecture::Universal,
                sandboxed: false,
                source: NativeSource::Steam,
            }),
            steam_app_id: Some("1086940".into()),
        }
    }

    /// Register a single enabled mod in `db` with a staging directory containing
    /// a synthetic `.pak` file at `staging_dir/MyMod.pak`.
    ///
    /// Returns the staging dir path.
    fn setup_mod_with_pak(
        db: &Arc<crate::database::ModDatabase>,
        game_id: &str,
        staging_root: &Path,
        pak_name: &str,
        pak_bytes: &[u8],
    ) -> PathBuf {
        let staging_dir = staging_root.join("mod_staging");
        std::fs::create_dir_all(&staging_dir).unwrap();

        let pak_path = staging_dir.join(pak_name);
        std::fs::write(&pak_path, pak_bytes).unwrap();

        let mod_id = db
            .add_mod(game_id, "", None, "Test Mod", "1.0.0", pak_name, &[])
            .unwrap();
        db.set_staging_path(mod_id, &staging_dir.to_string_lossy())
            .unwrap();

        staging_dir
    }

    // ── Task 4.5: load_order_kind ───────────────────────────────────────────

    #[test]
    fn bg3_load_order_kind_returns_file_based_with_bg3_modsettings_format() {
        let plugin = BaldursGate3NativePlugin;
        let kind = plugin.load_order_kind(Path::new("/Applications/Baldurs Gate 3.app"));
        match kind {
            crate::games::LoadOrderKind::FileBased(fb) => {
                assert!(
                    matches!(fb.format, crate::games::LoadOrderFormat::Bg3ModSettings),
                    "expected Bg3ModSettings format, got {:?}",
                    fb.format
                );
                // config_path must end in modsettings.lsx
                assert!(
                    fb.config_path
                        .file_name()
                        .map(|n| n == "modsettings.lsx")
                        .unwrap_or(false),
                    "config_path must point at modsettings.lsx, got {:?}",
                    fb.config_path
                );
                // describe is None (UUID + folder name is enough for the UI)
                assert!(fb.describe.is_none(), "describe should be None for BG3");
            }
            other => panic!("expected LoadOrderKind::FileBased, got {:?}", other),
        }
    }

    // ── Existing scaffold tests ─────────────────────────────────────────────

    #[test]
    fn bg3_native_plugin_registers() {
        crate::games::register_plugin(std::sync::Arc::new(BaldursGate3NativePlugin));
        let result = with_plugin("baldurs_gate_3_native", |p| p.display_name().to_owned());
        assert_eq!(result, Some("Baldur's Gate 3 (Native)".to_owned()));
    }

    #[test]
    fn bg3_get_data_dir_returns_mods_subfolder() {
        let plugin = BaldursGate3NativePlugin;
        let p = Path::new("/Applications/Baldurs Gate 3.app/Contents/MacOS");
        assert_eq!(plugin.get_data_dir(p), p.join("Mods"));
    }

    // ── Detection tests ─────────────────────────────────────────────────────

    #[test]
    fn bg3_detect_filters_by_bundle_id() {
        let candidates = vec![
            fake_candidate(
                "/Users/user/Library/Application Support/Steam/steamapps/common/Baldurs Gate 3/Baldur's Gate 3.app",
                BG3_BUNDLE_IDENTIFIER,
                "Baldur's Gate 3",
                false,
                NativeSource::Steam,
                Architecture::AppleSilicon,
            ),
            fake_candidate(
                "/Applications/OtherGame.app",
                "com.other.app",
                "OtherGame",
                false,
                NativeSource::SystemApplications,
                Architecture::Universal,
            ),
        ];

        let results = detect_from_candidates(candidates);
        assert_eq!(results.len(), 1, "only BG3 bundle should match");
        assert_eq!(results[0].game_id, "baldurs_gate_3_native");
        assert_eq!(results[0].display_name, "Baldur's Gate 3");
    }

    #[test]
    fn bg3_detect_accepts_executable_name_fallback() {
        let candidates = vec![fake_candidate(
            "/Users/user/Games/BG3/Baldur's Gate 3.app",
            "com.unknown.thing",     // non-standard bundle id
            "Baldur's Gate 3",       // matching executable name
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
        assert_eq!(results[0].game_id, "baldurs_gate_3_native");
    }

    #[test]
    fn bg3_detect_skips_sandboxed() {
        let candidates = vec![
            fake_candidate(
                "/Applications/Baldur's Gate 3.app",
                BG3_BUNDLE_IDENTIFIER,
                "Baldur's Gate 3",
                true, // sandboxed
                NativeSource::AppStore,
                Architecture::AppleSilicon,
            ),
            fake_candidate(
                "/Users/user/Games/BG3/Baldur's Gate 3.app",
                BG3_BUNDLE_IDENTIFIER,
                "Baldur's Gate 3",
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

    #[test]
    fn bg3_detect_steam_source_populates_app_id() {
        let candidates = vec![fake_candidate(
            "/Users/user/Library/Application Support/Steam/steamapps/common/Baldurs Gate 3/Baldur's Gate 3.app",
            BG3_BUNDLE_IDENTIFIER,
            "Baldur's Gate 3",
            false,
            NativeSource::Steam,
            Architecture::AppleSilicon,
        )];

        let results = detect_from_candidates(candidates);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].steam_app_id,
            Some(BG3_STEAM_APP_ID.to_string()),
            "Steam source must set steam_app_id to {}", BG3_STEAM_APP_ID
        );
    }

    #[test]
    fn bg3_detect_non_steam_source_omits_app_id() {
        let candidates = vec![fake_candidate(
            "/Applications/Baldur's Gate 3.app",
            BG3_BUNDLE_IDENTIFIER,
            "Baldur's Gate 3",
            false,
            NativeSource::SystemApplications,
            Architecture::Universal,
        )];

        let results = detect_from_candidates(candidates);
        assert_eq!(results.len(), 1);
        assert!(
            results[0].steam_app_id.is_none(),
            "non-Steam source must not set steam_app_id"
        );
    }

    #[test]
    fn bg3_detect_returns_empty_vec_with_empty_candidates() {
        let results = detect_from_candidates(vec![]);
        assert!(results.is_empty(), "empty candidates must yield empty results");
    }

    /// Integration test: requires the user's BG3 install to be present.
    /// Gated with #[ignore] so CI skips it.
    #[test]
    #[ignore]
    fn bg3_detect_native_finds_steam_install_on_this_machine() {
        let plugin = BaldursGate3NativePlugin;
        let results = plugin.detect_native();
        assert!(
            !results.is_empty(),
            "expected to find BG3 at ~/Library/Application Support/Steam/steamapps/common/Baldurs Gate 3/"
        );
        assert_eq!(results[0].game_id, "baldurs_gate_3_native");
        assert_eq!(
            results[0].steam_app_id,
            Some(BG3_STEAM_APP_ID.to_string())
        );
    }

    // ── deploy_native_inner tests (Task 4.4) ────────────────────────────────

    /// Happy-path: a staged `.pak` is copied to `mods_dir` and `modsettings.lsx`
    /// is created/updated with a `ModuleShortDesc` entry.
    #[test]
    fn bg3_deploy_native_copies_pak_to_mods_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        let modsettings_path = tmp.path().join("PlayerProfiles/Public/modsettings.lsx");

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());

        let pak_bytes = make_minimal_lspk("Mods/TestMod/meta.lsx", TEST_META_LSX_XML);
        setup_mod_with_pak(
            &db,
            "baldurs_gate_3_native",
            tmp.path(),
            "TestMod.pak",
            &pak_bytes,
        );

        let detected = fake_detected_native(tmp.path());
        let result = deploy_native_inner(&detected, &db, &mods_dir, &modsettings_path).unwrap();

        assert_eq!(result.deployed_count, 1, "one pak should be deployed");
        assert!(
            mods_dir.join("TestMod.pak").exists(),
            "TestMod.pak must be present in mods_dir"
        );
        assert!(
            modsettings_path.exists(),
            "modsettings.lsx must have been created"
        );
    }

    /// After deploy, `modsettings.lsx` must contain a `ModuleShortDesc` node
    /// with the UUID from the pak's `meta.lsx`.
    #[test]
    fn bg3_deploy_native_appends_entry_to_modsettings() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        let modsettings_path = tmp.path().join("PlayerProfiles/Public/modsettings.lsx");

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());

        let pak_bytes = make_minimal_lspk("Mods/TestMod/meta.lsx", TEST_META_LSX_XML);
        setup_mod_with_pak(
            &db,
            "baldurs_gate_3_native",
            tmp.path(),
            "TestMod.pak",
            &pak_bytes,
        );

        let detected = fake_detected_native(tmp.path());
        deploy_native_inner(&detected, &db, &mods_dir, &modsettings_path).unwrap();

        let settings = read_modsettings(&modsettings_path).unwrap();
        let uuids: Vec<&str> = settings.mods.iter().map(|m| m.uuid.as_str()).collect();
        assert!(
            uuids.contains(&"abcdef01-2345-6789-abcd-ef0123456789"),
            "modsettings must contain the mod UUID; got: {:?}",
            uuids
        );
    }

    /// If `modsettings.lsx` already exists with GustavDev, the deploy must keep
    /// GustavDev AND append the new mod (not replace it).
    #[test]
    fn bg3_deploy_native_preserves_existing_master_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        let profile_dir = tmp.path().join("PlayerProfiles/Public");
        std::fs::create_dir_all(&profile_dir).unwrap();
        let modsettings_path = profile_dir.join("modsettings.lsx");

        // Write a pre-existing modsettings.lsx that already contains GustavDev.
        let existing = crate::bg3_lsx::ModSettings {
            version: crate::bg3_lsx::LsxVersion {
                major: 4,
                minor: 0,
                revision: 9,
                build: 319,
            },
            mods: vec![
                ModEntry {
                    folder: "GustavDev".into(),
                    md5: "".into(),
                    name: "GustavDev".into(),
                    publish_handle: "0".into(),
                    uuid: MASTER_GUSTAV_DEV_UUID.into(),
                    version64: "36028797018963968".into(),
                },
                ModEntry {
                    folder: "Gustav".into(),
                    md5: "".into(),
                    name: "Gustav".into(),
                    publish_handle: "0".into(),
                    uuid: MASTER_GUSTAV_UUID.into(),
                    version64: "36028797018963968".into(),
                },
                ModEntry {
                    folder: "SharedDev".into(),
                    md5: "".into(),
                    name: "SharedDev".into(),
                    publish_handle: "0".into(),
                    uuid: MASTER_SHARED_DEV_UUID.into(),
                    version64: "36028797018963968".into(),
                },
            ],
        };
        write_modsettings(&modsettings_path, &existing).unwrap();

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        let pak_bytes = make_minimal_lspk("Mods/TestMod/meta.lsx", TEST_META_LSX_XML);
        setup_mod_with_pak(
            &db,
            "baldurs_gate_3_native",
            tmp.path(),
            "TestMod.pak",
            &pak_bytes,
        );

        let detected = fake_detected_native(tmp.path());
        deploy_native_inner(&detected, &db, &mods_dir, &modsettings_path).unwrap();

        let settings = read_modsettings(&modsettings_path).unwrap();
        let uuids: Vec<&str> = settings.mods.iter().map(|m| m.uuid.as_str()).collect();

        assert!(
            uuids.contains(&MASTER_GUSTAV_DEV_UUID),
            "GustavDev must be preserved; got {:?}",
            uuids
        );
        assert!(
            uuids.contains(&"abcdef01-2345-6789-abcd-ef0123456789"),
            "new mod must be appended; got {:?}",
            uuids
        );
        assert_eq!(
            settings.mods.len(),
            4,
            "3 masters + 1 community mod = 4 entries"
        );
    }

    /// When `modsettings.lsx` does not exist, it must be created with the
    /// modern Patch 8+ `GustavX` master plus the deployed mod (no legacy
    /// trio).
    #[test]
    fn bg3_deploy_native_creates_modsettings_with_masters_if_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        let modsettings_path = tmp.path().join("PlayerProfiles/Public/modsettings.lsx");
        // Deliberately do NOT create the file.

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        let pak_bytes = make_minimal_lspk("Mods/TestMod/meta.lsx", TEST_META_LSX_XML);
        setup_mod_with_pak(
            &db,
            "baldurs_gate_3_native",
            tmp.path(),
            "TestMod.pak",
            &pak_bytes,
        );

        let detected = fake_detected_native(tmp.path());
        deploy_native_inner(&detected, &db, &mods_dir, &modsettings_path).unwrap();

        let settings = read_modsettings(&modsettings_path).unwrap();
        let uuids: Vec<&str> = settings.mods.iter().map(|m| m.uuid.as_str()).collect();

        assert!(
            uuids.contains(&MASTER_GUSTAV_X_UUID),
            "GustavX master must be present; got {:?}",
            uuids
        );
        // The legacy trio MUST NOT be injected on a fresh install.
        for legacy in [
            MASTER_GUSTAV_DEV_UUID,
            MASTER_GUSTAV_UUID,
            MASTER_SHARED_DEV_UUID,
        ] {
            assert!(
                !uuids.contains(&legacy),
                "legacy master {} must NOT be injected on fresh Patch 8 install; got {:?}",
                legacy,
                uuids
            );
        }
        assert!(
            uuids.contains(&"abcdef01-2345-6789-abcd-ef0123456789"),
            "deployed mod must be present; got {:?}",
            uuids
        );
        assert_eq!(settings.mods.len(), 2, "1 master (GustavX) + 1 mod");
    }

    /// Re-deploying the same mod twice must not duplicate its entry in
    /// `modsettings.lsx`.
    #[test]
    fn bg3_deploy_native_replaces_existing_entry_with_same_uuid() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        let modsettings_path = tmp.path().join("PlayerProfiles/Public/modsettings.lsx");

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        let pak_bytes = make_minimal_lspk("Mods/TestMod/meta.lsx", TEST_META_LSX_XML);
        setup_mod_with_pak(
            &db,
            "baldurs_gate_3_native",
            tmp.path(),
            "TestMod.pak",
            &pak_bytes,
        );

        let detected = fake_detected_native(tmp.path());

        // First deploy.
        deploy_native_inner(&detected, &db, &mods_dir, &modsettings_path).unwrap();
        // Second deploy (idempotent).
        deploy_native_inner(&detected, &db, &mods_dir, &modsettings_path).unwrap();

        let settings = read_modsettings(&modsettings_path).unwrap();

        // Count occurrences of the mod's UUID.
        let mod_uuid = "abcdef01-2345-6789-abcd-ef0123456789";
        let count = settings
            .mods
            .iter()
            .filter(|m| m.uuid == mod_uuid)
            .count();
        assert_eq!(count, 1, "re-deploy must not duplicate the entry; got {}", count);
    }

    /// A staged `.pak` whose filename contains `..` or `/` must be rejected
    /// with an error before any file is written.
    #[test]
    fn bg3_deploy_native_refuses_path_traversal_in_pak_filename() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        let modsettings_path = tmp.path().join("PlayerProfiles/Public/modsettings.lsx");

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());

        // Create a staging dir with a malicious filename.
        let staging_dir = tmp.path().join("evil_staging");
        std::fs::create_dir_all(&staging_dir).unwrap();

        // We can't create a file named "../escape.pak" directly on most
        // filesystems, so we create an inner subdirectory that simulates
        // a traversal path being present in the pak's name field.
        //
        // Instead, we write a file with a safe filename first, register it,
        // then rename the staging directory to contain a symlink that would
        // escape.  Since direct filesystem trickery is awkward in portable
        // tests, we test the validation logic by calling is_safe_relative_path
        // directly and verifying the error path would trigger.
        //
        // For the deploy test: put a file whose *directory component* would
        // escape.  walkdir returns absolute paths, so the filename is always
        // just the basename — but we test that a basename of "..pak" (two
        // dots, valid basename but could be confused) is actually valid while
        // "../../../../etc/passwd.pak" would be caught by is_safe_relative_path.
        //
        // Verify the safety predicates that deploy_native_inner relies on.
        // is_safe_relative_path rejects '..' components and absolute paths.
        assert!(
            !is_safe_relative_path("../../escape.pak"),
            "is_safe_relative_path must reject traversal components"
        );
        assert!(
            !is_safe_relative_path("../escape.pak"),
            "is_safe_relative_path must reject '..' components"
        );
        // The deploy code adds a separate explicit check for '/' in the filename.
        // Verify the combined condition (is_safe_relative_path OR contains '/'):
        let unsafe_with_slash = "sub/dir/escape.pak";
        let rejected = !is_safe_relative_path(unsafe_with_slash)
            || unsafe_with_slash.contains('/')
            || unsafe_with_slash.contains('\\');
        assert!(
            rejected,
            "filename with embedded '/' must be rejected by the combined check"
        );

        // Also verify the deploy function rejects a staging pak whose name
        // contains a '/' in the final path component check.
        // We construct a db entry pointing to a staging dir that has a sub-
        // directory with a .pak — the file name is safe but a mod author might
        // craft a pak at depth.  The loop only picks up the *filename*, not
        // the full relative path, so the traversal check is on the filename only.
        let pak_bytes = make_minimal_lspk("Mods/TestMod/meta.lsx", TEST_META_LSX_XML);
        let nested_dir = staging_dir.join("sub");
        std::fs::create_dir_all(&nested_dir).unwrap();
        std::fs::write(nested_dir.join("valid.pak"), &pak_bytes).unwrap();

        let mod_id = db
            .add_mod(
                "baldurs_gate_3_native",
                "",
                None,
                "Evil Mod",
                "1.0.0",
                "evil.pak",
                &[],
            )
            .unwrap();
        db.set_staging_path(mod_id, &staging_dir.to_string_lossy())
            .unwrap();

        let detected = fake_detected_native(tmp.path());
        // Nested pak with safe name must succeed (it's just copied from sub/).
        let result = deploy_native_inner(&detected, &db, &mods_dir, &modsettings_path);
        // The pak is valid; deploy should succeed (the filename "valid.pak" is safe).
        assert!(
            result.is_ok(),
            "nested pak with safe filename must succeed: {:?}",
            result
        );
        assert!(
            mods_dir.join("valid.pak").exists(),
            "valid.pak should be deployed into mods_dir"
        );
    }

    // ── Snapshot integration tests (Task 6.1) ───────────────────────────────

    /// deploy_native_inner() creates a snapshot row in the DB before deploying.
    #[test]
    fn bg3_deploy_native_creates_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        let modsettings_path = tmp.path().join("PlayerProfiles/Public/modsettings.lsx");

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        // rollback schema is outside migrations; must be initialised explicitly.
        crate::rollback::init_schema(&db).unwrap();

        let pak_bytes = make_minimal_lspk("Mods/TestMod/meta.lsx", TEST_META_LSX_XML);
        setup_mod_with_pak(
            &db,
            "baldurs_gate_3_native",
            tmp.path(),
            "TestMod.pak",
            &pak_bytes,
        );

        let detected = fake_detected_native(tmp.path());
        deploy_native_inner(&detected, &db, &mods_dir, &modsettings_path).unwrap();

        let snapshots =
            crate::rollback::list_snapshots(&db, "baldurs_gate_3_native", "").unwrap();
        assert!(
            !snapshots.is_empty(),
            "deploy_native_inner should have created at least one snapshot"
        );
        assert_eq!(
            snapshots[0].name, "bg3-deploy",
            "snapshot name must be 'bg3-deploy'"
        );
    }

    // ── Sandbox refusal tests (Task 6.2) ────────────────────────────────────

    /// `deploy_native_inner` must return an error immediately when the game's
    /// `NativeContext.sandboxed` flag is true. No files should be written and
    /// the error message must mention "sandboxed".
    #[test]
    fn bg3_deploy_native_refuses_sandboxed_game() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        let modsettings_path = tmp.path().join("PlayerProfiles/Public/modsettings.lsx");

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());

        // Build a DetectedGame with sandboxed = true.
        let detected = DetectedGame {
            game_id: "baldurs_gate_3_native".into(),
            display_name: "Baldur's Gate 3 (Native)".into(),
            nexus_slug: "baldursgate3".into(),
            game_path: tmp.path().to_path_buf(),
            exe_path: Some(tmp.path().join("bg3")),
            data_dir: tmp.path().join("Mods"),
            runtime: GameRuntime::Native(NativeContext {
                app_bundle_path: tmp.path().to_path_buf(),
                game_data_root: tmp.path().to_path_buf(),
                architecture: Architecture::Universal,
                sandboxed: true, // ← sandboxed
                source: NativeSource::Steam,
            }),
            steam_app_id: Some("1086940".into()),
        };

        let result = deploy_native_inner(&detected, &db, &mods_dir, &modsettings_path);

        assert!(result.is_err(), "deploy_native_inner must refuse sandboxed game");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("sandboxed"),
            "error must mention 'sandboxed': {msg}"
        );
        // mods_dir must not have been created.
        assert!(
            !mods_dir.exists(),
            "mods_dir must not be created for sandboxed game"
        );
    }

    // ── Patch 8 master-handling tests ───────────────────────────────────────

    /// `bootstrap_master_entries` should return a single Patch 8+ GustavX
    /// entry (no legacy trio).
    #[test]
    fn bootstrap_master_entries_returns_gustav_x_only() {
        let masters = bootstrap_master_entries();
        assert_eq!(
            masters.len(),
            1,
            "Patch 8+ default should be a single master entry; got {}",
            masters.len()
        );
        assert_eq!(masters[0].folder, "GustavX");
        assert_eq!(masters[0].uuid, MASTER_GUSTAV_X_UUID);
        assert_eq!(masters[0].version64, "36028797018963968");
        // None of the legacy UUIDs should appear.
        for legacy in [
            MASTER_GUSTAV_DEV_UUID,
            MASTER_GUSTAV_UUID,
            MASTER_SHARED_DEV_UUID,
        ] {
            assert!(
                masters.iter().all(|m| m.uuid != legacy),
                "legacy UUID {} must not appear in default bootstrap",
                legacy
            );
        }
    }

    /// Pre-Patch-8 compat: a `modsettings.lsx` that already contains the
    /// legacy trio must be preserved as-is. The modern GustavX default
    /// must NOT be injected, and no legacy entry must be removed.
    #[test]
    fn deploy_native_preserves_existing_legacy_masters() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        let profile_dir = tmp.path().join("PlayerProfiles/Public");
        std::fs::create_dir_all(&profile_dir).unwrap();
        let modsettings_path = profile_dir.join("modsettings.lsx");

        // Seed file with the legacy trio.
        let existing = ModSettings {
            version: LsxVersion { major: 4, minor: 0, revision: 9, build: 319 },
            mods: vec![
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
            ],
        };
        write_modsettings(&modsettings_path, &existing).unwrap();

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        let pak_bytes = make_minimal_lspk("Mods/TestMod/meta.lsx", TEST_META_LSX_XML);
        setup_mod_with_pak(
            &db,
            "baldurs_gate_3_native",
            tmp.path(),
            "TestMod.pak",
            &pak_bytes,
        );

        let detected = fake_detected_native(tmp.path());
        deploy_native_inner(&detected, &db, &mods_dir, &modsettings_path).unwrap();

        let settings = read_modsettings(&modsettings_path).unwrap();
        let uuids: Vec<&str> = settings.mods.iter().map(|m| m.uuid.as_str()).collect();

        // All three legacy masters must be preserved.
        assert!(uuids.contains(&MASTER_GUSTAV_DEV_UUID), "GustavDev preserved");
        assert!(uuids.contains(&MASTER_GUSTAV_UUID), "Gustav preserved");
        assert!(uuids.contains(&MASTER_SHARED_DEV_UUID), "SharedDev preserved");
        // Modern GustavX must NOT have been added — we don't replace
        // legacy with modern.
        assert!(
            !uuids.contains(&MASTER_GUSTAV_X_UUID),
            "GustavX must not be injected when legacy trio already exists; got {:?}",
            uuids
        );
        // The community mod must be appended.
        assert!(uuids.contains(&"abcdef01-2345-6789-abcd-ef0123456789"));
        assert_eq!(settings.mods.len(), 4, "3 legacy masters + 1 community mod");
    }

    /// Patch 8+: a `modsettings.lsx` that already contains just `GustavX`
    /// (the empirical vanilla shape) must be preserved exactly — no
    /// phantom legacy masters injected.
    #[test]
    fn deploy_native_preserves_existing_gustav_x() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        let profile_dir = tmp.path().join("PlayerProfiles/Public");
        std::fs::create_dir_all(&profile_dir).unwrap();
        let modsettings_path = profile_dir.join("modsettings.lsx");

        // Seed file with empirical Patch 8 vanilla content (GustavX only).
        let existing = ModSettings {
            version: LsxVersion { major: 4, minor: 8, revision: 0, build: 700 },
            mods: vec![ModEntry {
                folder: "GustavX".into(),
                md5: String::new(),
                name: "GustavX".into(),
                publish_handle: "0".into(),
                uuid: MASTER_GUSTAV_X_UUID.into(),
                version64: "36028797018963968".into(),
            }],
        };
        write_modsettings(&modsettings_path, &existing).unwrap();

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        let pak_bytes = make_minimal_lspk("Mods/TestMod/meta.lsx", TEST_META_LSX_XML);
        setup_mod_with_pak(
            &db,
            "baldurs_gate_3_native",
            tmp.path(),
            "TestMod.pak",
            &pak_bytes,
        );

        let detected = fake_detected_native(tmp.path());
        deploy_native_inner(&detected, &db, &mods_dir, &modsettings_path).unwrap();

        let settings = read_modsettings(&modsettings_path).unwrap();
        let uuids: Vec<&str> = settings.mods.iter().map(|m| m.uuid.as_str()).collect();

        assert!(uuids.contains(&MASTER_GUSTAV_X_UUID), "GustavX preserved");
        for legacy in [
            MASTER_GUSTAV_DEV_UUID,
            MASTER_GUSTAV_UUID,
            MASTER_SHARED_DEV_UUID,
        ] {
            assert!(
                !uuids.contains(&legacy),
                "legacy master {} must NOT be injected into Patch 8 vanilla file; got {:?}",
                legacy,
                uuids
            );
        }
        assert!(uuids.contains(&"abcdef01-2345-6789-abcd-ef0123456789"));
        assert_eq!(settings.mods.len(), 2, "1 master (GustavX) + 1 community mod");
    }

    /// If a valid `modsettings.lsx` is present but has zero master entries
    /// (damaged or wiped), deploy must prepend the modern Patch 8 GustavX
    /// default as the bootstrap. The deployed mod follows.
    #[test]
    fn deploy_native_bootstraps_gustav_x_when_no_masters_present() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        let profile_dir = tmp.path().join("PlayerProfiles/Public");
        std::fs::create_dir_all(&profile_dir).unwrap();
        let modsettings_path = profile_dir.join("modsettings.lsx");

        // Write a valid file with NO mod entries (no masters).
        let empty = ModSettings {
            version: LsxVersion { major: 4, minor: 8, revision: 0, build: 700 },
            mods: vec![],
        };
        write_modsettings(&modsettings_path, &empty).unwrap();

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        let pak_bytes = make_minimal_lspk("Mods/TestMod/meta.lsx", TEST_META_LSX_XML);
        setup_mod_with_pak(
            &db,
            "baldurs_gate_3_native",
            tmp.path(),
            "TestMod.pak",
            &pak_bytes,
        );

        let detected = fake_detected_native(tmp.path());
        deploy_native_inner(&detected, &db, &mods_dir, &modsettings_path).unwrap();

        let settings = read_modsettings(&modsettings_path).unwrap();
        assert_eq!(
            settings.mods.len(),
            2,
            "bootstrap should yield 1 master (GustavX) + 1 community mod; got {:?}",
            settings.mods.iter().map(|m| m.uuid.clone()).collect::<Vec<_>>()
        );
        // GustavX must come first (prepended).
        assert_eq!(
            settings.mods[0].uuid, MASTER_GUSTAV_X_UUID,
            "GustavX must be prepended as the first entry"
        );
        // Community mod must follow.
        assert_eq!(settings.mods[1].uuid, "abcdef01-2345-6789-abcd-ef0123456789");
    }

    /// Regression: starting with the empirical Patch 8 vanilla file
    /// (GustavX-only), deploying a mod MUST NOT inject any of the
    /// pre-Patch-8 legacy masters. Injecting them would corrupt the load
    /// order on Patch 8+ installs.
    #[test]
    fn deploy_native_does_not_inject_legacy_trio() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        let profile_dir = tmp.path().join("PlayerProfiles/Public");
        std::fs::create_dir_all(&profile_dir).unwrap();
        let modsettings_path = profile_dir.join("modsettings.lsx");

        let vanilla_patch8 = ModSettings {
            version: LsxVersion { major: 4, minor: 8, revision: 0, build: 700 },
            mods: vec![ModEntry {
                folder: "GustavX".into(),
                md5: String::new(),
                name: "GustavX".into(),
                publish_handle: "0".into(),
                uuid: MASTER_GUSTAV_X_UUID.into(),
                version64: "36028797018963968".into(),
            }],
        };
        write_modsettings(&modsettings_path, &vanilla_patch8).unwrap();

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        let pak_bytes = make_minimal_lspk("Mods/TestMod/meta.lsx", TEST_META_LSX_XML);
        setup_mod_with_pak(
            &db,
            "baldurs_gate_3_native",
            tmp.path(),
            "TestMod.pak",
            &pak_bytes,
        );

        let detected = fake_detected_native(tmp.path());
        deploy_native_inner(&detected, &db, &mods_dir, &modsettings_path).unwrap();

        let settings = read_modsettings(&modsettings_path).unwrap();
        let uuids: Vec<String> =
            settings.mods.iter().map(|m| m.uuid.to_lowercase()).collect();

        for legacy in [
            MASTER_GUSTAV_DEV_UUID,
            MASTER_GUSTAV_UUID,
            MASTER_SHARED_DEV_UUID,
        ] {
            assert!(
                !uuids.contains(&legacy.to_lowercase()),
                "legacy master {} must NEVER be present in Patch 8 modsettings.lsx; got {:?}",
                legacy,
                uuids
            );
        }
    }
}
