//! Crimson Desert (native macOS) game plugin.
//!
//! Pearl Abyss's open-world action RPG, native Apple Silicon build
//! released March 19, 2026 (macOS 15.0+ required). Bundle identifier
//! `com.pearlabyss.CrimsonDesert`, Steam App ID `3321460`.
//!
//! Architecture: Apple Silicon ONLY. Intel users on Rosetta will NOT
//! see this plugin produce a detection — the spike confirmed no Intel
//! binary slice is shipped.
//!
//! Modding posture: Pearl Abyss unofficially tolerates client-side
//! mods. The game ships Denuvo Anti-Tamper (DRM only — no XignCode/EQU8
//! anti-cheat). Mods carry user-acceptance risk under the EULA but
//! there are no documented ban incidents and the offline mode insulates
//! the user from server-side detection.
//!
//! Mod architecture: PAZ overlay groups numbered `0036+` (vanilla ships
//! `0000`–`0035`). A mod ships a pre-built overlay directory containing
//! `<group>/0.paz` + `<group>/0.pamt` and the registration is appended
//! to `meta/0.papgt`. This Corkscrew plugin implements the deploy
//! orchestration; the PAZ format itself is opaque to us — we never read
//! or write inside `.paz` archives. Format-aware mod tooling (JSON
//! byte-patches that need PAZ extraction) is a separate future project,
//! not Phase 1.
//!
//! DEPLOY IS BLOCKED on verifying the PAZ overlay tree location: does
//! it live at `<game_install>/Paz/` (writable, no signing impact) or
//! inside `.app/Contents/Resources/` (writing breaks code signing)?
//! Until verified on a real install, deploy_native returns a typed
//! BLOCKED error explaining the situation.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::bottles::Bottle;
use crate::database::ModDatabase;
use crate::deployer::{DeployResult, DeployerError};
use crate::games::{DetectedGame, GamePlugin};
use crate::staging::is_safe_relative_path;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Gate flag for the Crimson Desert native deploy pipeline.
///
/// While `false`, [`CrimsonDesertNativePlugin::deploy_native`] returns the
/// BLOCKED error and refuses to touch the filesystem, but the underlying
/// [`deploy_native_inner`] implementation is fully present and tested via the
/// unit test module. Flip to `true` ONLY after **all** of the following are
/// confirmed:
///
/// 1. **Install layout verified on a real macOS Crimson Desert install.** PAZ
///    overlay groups live at `<game_install>/<NNNN>/0.paz` + `<NNNN>/0.pamt`
///    and `meta/0.papgt` lives at `<game_install>/meta/0.papgt` — both outside
///    the `.app` bundle, so the Apple Developer ID signature is preserved.
///    See `docs/superpowers/plans/2026-05-02-crimson-desert-native-spec.md`
///    §10.5 and the standing verification procedure for the exact paths to
///    check before flipping this.
/// 2. **PAPGT registry format reverse-engineered.** Copying overlay files into
///    `<game_install>/<NNNN>/` is necessary but not sufficient — the engine
///    only loads groups listed in `meta/0.papgt`. Until [`PapgtEditor`] can
///    safely append entries (Phase 1b), enabling deploy would silently produce
///    a "deployed but not loaded" outcome.
/// 3. **Frontend anti-cheat warning surfaced.** The spec §9 Step 6 calls for a
///    pre-deploy notice screen explaining the EULA / online-mode risk. That
///    component lives in `src/routes/native/` and is tracked separately —
///    deploy must not run before the user has accepted that warning.
/// 4. **Snapshot/restore round-trip tested end-to-end** with at least one
///    real overlay mod deploy + uninstall + revert-to-vanilla cycle.
const VERIFIED: bool = false;

const CD_BUNDLE_IDENTIFIER: &str = "com.pearlabyss.CrimsonDesert";

/// Candidate executable names inside `Contents/MacOS/`.
///
/// The authoritative name is an open spike question (see spec open question #2).
/// These cover known possibilities from community tools and Pearl Abyss naming
/// conventions — `CrimsonDesert_Steam` is a community-observed Steam wrapper
/// variant that may or may not exist. Refine after verifying a real install.
///
/// TODO: verify with real install — run `ls "<app>/Contents/MacOS/"`.
const CD_BUNDLE_EXECUTABLE: &str = "Crimson Desert";

const CD_STEAM_APP_ID: &str = "3321460";

/// Bottle sentinel for native mods (no Wine bottle).
const CD_NATIVE_BOTTLE_SENTINEL: &str = "";

/// First overlay group number available to mods. Vanilla ships `0000`–`0035`.
const CD_FIRST_MOD_GROUP: u16 = 36;

/// Width of the zero-padded overlay group directory name (e.g. `0036`).
const CD_GROUP_NAME_WIDTH: usize = 4;

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

pub struct CrimsonDesertNativePlugin;

// ---------------------------------------------------------------------------
// Detection helper (testable)
// ---------------------------------------------------------------------------

/// Core detection logic. Accepts a pre-scanned candidate list so unit tests
/// can inject synthetic candidates without touching the real filesystem.
fn detect_from_candidates(
    candidates: Vec<crate::native_scanner::NativeAppCandidate>,
) -> Vec<DetectedGame> {
    candidates
        .into_iter()
        // Sandbox refusal: App Store sandboxed bundles cannot be modded.
        // The sandboxed game writes into its own container; Corkscrew cannot
        // reach that container without violating the App Sandbox.
        .filter(|c| !c.sandboxed)
        // Apple Silicon only — no Intel build exists. If we somehow encounter
        // an Intel-only candidate (unexpected), skip it rather than producing a
        // DetectedGame that would fail at deploy time.
        .filter(|c| c.architecture != crate::runtime::Architecture::IntelOnly)
        .filter(|c| {
            // Case-insensitive — Info.plist authors are inconsistent about casing.
            c.info.bundle_identifier.eq_ignore_ascii_case(CD_BUNDLE_IDENTIFIER)
                || c.info.bundle_executable.eq_ignore_ascii_case(CD_BUNDLE_EXECUTABLE)
                // Also accept the no-space community variant.
                || c.info.bundle_executable.eq_ignore_ascii_case("CrimsonDesert")
        })
        .map(|c| {
            // Game install root: for Steam, the .app lives inside the Steam
            // common dir, e.g. `common/Crimson Desert/CrimsonDesert_Steam.app`.
            // The PAZ group tree is expected at the *parent* of the .app bundle
            // (the install root), not inside the bundle itself.
            //
            // OPEN QUESTION (spike §10.3 / §10.5): if PAZ groups turn out to
            // live inside `Contents/Resources/`, this mapping must be revised.
            // Until then, install_root = .app parent is the working assumption.
            let install_root = c
                .bundle_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| c.bundle_path.clone());

            let game_path = install_root.clone();

            // Executable path inside the bundle's MacOS directory.
            let exe_path = Some(
                c.bundle_path
                    .join("Contents")
                    .join("MacOS")
                    .join(&c.info.bundle_executable),
            );

            // PAZ overlay tree lives at <game_install>/Paz/ — pending verification.
            // See spec open question #5: if groups are inside Contents/Resources/
            // this breaks code signing and the deploy path must be redesigned.
            let data_dir = game_path.join("Paz");

            let steam_app_id = if c.source == crate::runtime::NativeSource::Steam {
                Some(CD_STEAM_APP_ID.to_string())
            } else {
                None
            };

            DetectedGame {
                game_id: "crimson_desert_native".to_string(),
                display_name: "Crimson Desert".to_string(),
                nexus_slug: "crimsondesert".to_string(),
                game_path,
                exe_path,
                data_dir,
                runtime: crate::runtime::GameRuntime::Native(crate::runtime::NativeContext {
                    app_bundle_path: c.bundle_path,
                    game_data_root: install_root,
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
// GamePlugin impl
// ---------------------------------------------------------------------------

impl GamePlugin for CrimsonDesertNativePlugin {
    fn game_id(&self) -> &str {
        "crimson_desert_native"
    }

    fn display_name(&self) -> &str {
        "Crimson Desert (Native)"
    }

    fn nexus_slug(&self) -> &str {
        "crimsondesert"
    }

    fn executables(&self) -> &[&str] {
        // Executable names inside Contents/MacOS/. Space variant is the
        // typical Pearl Abyss macOS naming; no-space variant is a community-
        // observed fallback. See spike open question #2 for TODO.
        &["Crimson Desert", "CrimsonDesert"]
    }

    /// Wine detection is not applicable — this is a native-only plugin.
    fn detect_wine(&self, _bottle: &Bottle) -> Option<DetectedGame> {
        None
    }

    fn detect_native(&self) -> Vec<DetectedGame> {
        detect_from_candidates(crate::native_scanner::scan_all_native())
    }

    /// Returns the PAZ overlay tree root relative to the game install path.
    ///
    /// Working assumption: `<game_install>/Paz/`. If PAZ groups live inside
    /// `.app/Contents/Resources/` (spike open question #5), this must be
    /// updated — writing into the bundle would break code signing and require
    /// a different deploy strategy.
    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        game_path.join("Paz")
    }

    /// Crimson Desert has no plugin/load-order manifest.
    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        None
    }

    /// Native deploy — gated behind [`VERIFIED`].
    ///
    /// While `VERIFIED == false`, this returns the typed BLOCKED error
    /// regardless of mod state. The full pipeline is implemented in
    /// [`deploy_native_inner`] and exercised by the test module — flipping
    /// `VERIFIED` to `true` activates it.
    fn deploy_native(
        &self,
        detected: &DetectedGame,
        db: &Arc<ModDatabase>,
    ) -> std::result::Result<DeployResult, DeployerError> {
        let native = detected
            .runtime
            .native()
            .ok_or_else(|| DeployerError::Other("expected native runtime".into()))?;

        // Sandbox refusal at the deploy layer (belt-and-suspenders — detection
        // already filters sandboxed candidates, but a DetectedGame could arrive
        // via a manual path).
        if native.sandboxed {
            return Err(DeployerError::Other(format!(
                "native modding refused for sandboxed app: {}. \
                 Use the Steam version of Crimson Desert instead.",
                native.app_bundle_path.display()
            )));
        }

        // Apple Silicon-only check. No Intel build of Crimson Desert exists;
        // if IntelOnly somehow slips through, reject it explicitly so the error
        // is legible rather than silently deploying to a broken state.
        if native.architecture == crate::runtime::Architecture::IntelOnly {
            return Err(DeployerError::Other(
                "Crimson Desert is Apple Silicon only — Intel binary detected (unexpected). \
                 Rosetta 2 is not a supported mod target for this plugin."
                    .into(),
            ));
        }

        if !VERIFIED {
            return Err(DeployerError::Other(
                "Crimson Desert native deploy is implemented but gated off pending \
                 PAZ overlay path verification on a real install. See \
                 docs/superpowers/plans/2026-05-02-crimson-desert-native-spec.md \
                 §10.5 for the verification procedure and flip VERIFIED=true in \
                 plugins/crimson_desert_native.rs when confirmed."
                    .into(),
            ));
        }

        let install_root = resolve_game_install_root(detected);
        deploy_native_inner(detected, db, &install_root)
    }
}

// ---------------------------------------------------------------------------
// Install-root resolution
// ---------------------------------------------------------------------------

/// Resolve the Crimson Desert install root for deploy.
///
/// Matches spec §9 Step 3: prefer `NativeContext::game_data_root` (the resolved
/// parent of the `.app` bundle as captured at detection time), falling back to
/// `DetectedGame::game_path` for manually-registered installs that lack a
/// native runtime context.
pub fn resolve_game_install_root(detected: &DetectedGame) -> PathBuf {
    detected
        .runtime
        .native()
        .map(|n| n.game_data_root.clone())
        .unwrap_or_else(|| detected.game_path.clone())
}

// ---------------------------------------------------------------------------
// Overlay group number assignment
// ---------------------------------------------------------------------------

/// Scan `install_root` for existing numeric overlay group directories (e.g.
/// `0000`, `0001`, … `0035`) and return the next group number to assign.
///
/// Semantics:
/// - Considers only directory entries whose name is a 4-digit zero-padded
///   decimal number. Any other entry (`Paz/`, `meta/`, files, hidden dirs) is
///   ignored — the PAPGT loader does the same, per the community-confirmed
///   behavior captured in spec §6.
/// - Returns `max(existing) + 1`, with a floor of [`CD_FIRST_MOD_GROUP`]
///   (`0036`). We do NOT gap-fill — sequential assignment keeps deploy/restore
///   semantics simple and matches CDUMM's behavior on Windows.
/// - When `install_root` does not exist or cannot be read, returns the floor.
pub fn next_available_group_number(install_root: &Path) -> u16 {
    let mut max_seen: Option<u16> = None;
    if let Ok(entries) = std::fs::read_dir(install_root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name();
            let Some(name_str) = name.to_str() else { continue };
            if name_str.len() != CD_GROUP_NAME_WIDTH {
                continue;
            }
            if !name_str.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            if let Ok(n) = name_str.parse::<u16>() {
                max_seen = Some(max_seen.map_or(n, |m| m.max(n)));
            }
        }
    }
    match max_seen {
        Some(m) => (m + 1).max(CD_FIRST_MOD_GROUP),
        None => CD_FIRST_MOD_GROUP,
    }
}

/// Format a group number as the zero-padded directory name (e.g. `36` → `"0036"`).
fn format_group_name(n: u16) -> String {
    format!("{:0width$}", n, width = CD_GROUP_NAME_WIDTH)
}

// ---------------------------------------------------------------------------
// Mod staging classification
// ---------------------------------------------------------------------------

/// What kind of artifact a mod's staging directory contains.
///
/// Determined by [`classify_mod_staging`] — a pure, walkdir-based scan that
/// dispatches the deploy strategy without touching the destination.
#[derive(Debug, PartialEq, Eq)]
pub enum ModDeployKind {
    /// One or more `(NNNN/0.paz, NNNN/0.pamt)` pairs ready to copy into a new
    /// overlay group directory. The paths are the **absolute** staging paths
    /// of the paz and pamt files, captured in order.
    PreBuiltOverlay {
        paz_and_pamt_pairs: Vec<(PathBuf, PathBuf)>,
    },
    /// Loose assets (DDS, JSON, etc.) without a paz+pamt pair. Phase 1
    /// limitation per spec §9 Step 4 — skipped with a warn log.
    LooseAssets,
    /// Windows-only mod artifact (`.asi` / `.dll`). Rejected at deploy time.
    AsiDll(PathBuf),
    /// No files found.
    Empty,
}

/// Walk `staging_dir` and classify the mod's deploy kind.
///
/// Detection priority:
/// 1. Any `.asi` or `.dll` → [`ModDeployKind::AsiDll`] (Windows-only artifact,
///    must be reported as a hard error so the user knows to find a macOS
///    version).
/// 2. `.paz` files that have a sibling `.pamt` with the matching stem →
///    [`ModDeployKind::PreBuiltOverlay`]. The pair is kept in walkdir order.
/// 3. Any other files → [`ModDeployKind::LooseAssets`] (deferred to Phase 1+).
/// 4. No files at all → [`ModDeployKind::Empty`].
pub fn classify_mod_staging(staging_dir: &Path) -> ModDeployKind {
    let mut paz_files: Vec<PathBuf> = Vec::new();
    let mut pamt_files: Vec<PathBuf> = Vec::new();
    let mut other_files: Vec<PathBuf> = Vec::new();
    let mut asi_dll: Option<PathBuf> = None;

    for entry in walkdir::WalkDir::new(staging_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.into_path();
        let ext_lower = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase());
        match ext_lower.as_deref() {
            Some("asi") | Some("dll") => {
                // First-seen-wins for the error report; we only need one path
                // to make the rejection legible.
                if asi_dll.is_none() {
                    asi_dll = Some(path);
                }
            }
            Some("paz") => paz_files.push(path),
            Some("pamt") => pamt_files.push(path),
            _ => other_files.push(path),
        }
    }

    if let Some(p) = asi_dll {
        return ModDeployKind::AsiDll(p);
    }

    // Pair up paz+pamt by file stem (e.g. `0.paz` ↔ `0.pamt`).
    let mut pairs: Vec<(PathBuf, PathBuf)> = Vec::new();
    for paz in &paz_files {
        let Some(stem) = paz.file_stem().and_then(|s| s.to_str()) else { continue };
        let paz_parent = paz.parent();
        if let Some(pamt) = pamt_files.iter().find(|p| {
            p.file_stem().and_then(|s| s.to_str()) == Some(stem) && p.parent() == paz_parent
        }) {
            pairs.push((paz.clone(), pamt.clone()));
        }
    }

    if !pairs.is_empty() {
        return ModDeployKind::PreBuiltOverlay {
            paz_and_pamt_pairs: pairs,
        };
    }

    if paz_files.is_empty() && pamt_files.is_empty() && other_files.is_empty() {
        ModDeployKind::Empty
    } else {
        ModDeployKind::LooseAssets
    }
}

// ---------------------------------------------------------------------------
// PAPGT editor — Phase 1b stub
// ---------------------------------------------------------------------------

/// PAPGT registry editor.
///
/// The on-disk PAPGT format is opaque to Corkscrew today: the spec (§9 Step 4)
/// calls it a "small binary/text edit" but does not document a layout we can
/// safely round-trip. Phase 1a (this code) backs up `meta/0.papgt` to
/// `meta/0.papgt.bak` so that any future Phase 1b write can be reverted, but
/// the write itself is deliberately stubbed — copying overlay files without
/// registering them in PAPGT means the engine won't load them, but it also
/// means we never corrupt the registry while the format is uncertain.
///
/// When format work lands (from NattKh's CrimsonDesertModdingTools or a
/// verification pass on a real install), implement [`PapgtEditor::register_group`]
/// to append entries here.
pub struct PapgtEditor;

impl PapgtEditor {
    /// Back up `<install_root>/meta/0.papgt` to `meta/0.papgt.bak` if (a) the
    /// original exists and (b) no backup is present yet. Existing backups are
    /// preserved byte-for-byte — the first deploy captures vanilla; subsequent
    /// deploys must not overwrite that snapshot.
    pub fn ensure_backup(install_root: &Path) -> Result<(), DeployerError> {
        let papgt = install_root.join("meta").join("0.papgt");
        let backup = install_root.join("meta").join("0.papgt.bak");
        if backup.exists() {
            return Ok(());
        }
        if !papgt.exists() {
            // Nothing to back up — first run on a fresh install or a path that
            // doesn't ship PAPGT (unexpected, but not a deploy-blocking error).
            return Ok(());
        }
        std::fs::copy(&papgt, &backup)
            .map(|_| ())
            .map_err(|e| DeployerError::Other(format!("backup 0.papgt: {}", e)))
    }

    /// Register a freshly-deployed overlay group in `meta/0.papgt`.
    ///
    /// **Phase 1b — not yet implemented.** The on-disk format is unknown; see
    /// the [`PapgtEditor`] doc-comment. Returning `Err` here keeps the gated
    /// pipeline honest: if [`VERIFIED`] is flipped without the format work, the
    /// deploy fails loudly rather than silently producing dead overlays.
    pub fn register_group(_install_root: &Path, _group: u16) -> Result<(), DeployerError> {
        Err(DeployerError::Other(
            "PAPGT registration is not implemented (Phase 1b). \
             Overlay files were copied but the meta/0.papgt registry was not \
             updated, so the engine will not load them. Implement \
             PapgtEditor::register_group after reverse-engineering the format \
             from NattKh/CrimsonDesertModdingTools or a real-install verification."
                .into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Single-file copy helper
// ---------------------------------------------------------------------------

/// Hardlink-first, copy-on-fallback file deploy with traversal containment.
fn deploy_one_file(src: &Path, dest: &Path) -> Result<(), DeployerError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| DeployerError::Other(format!("create dest parent: {}", e)))?;
    }
    if dest.exists() {
        let _ = std::fs::remove_file(dest);
    }
    if std::fs::hard_link(src, dest).is_err() {
        std::fs::copy(src, dest)
            .map_err(|e| DeployerError::Other(format!("copy overlay file: {}", e)))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Inner deploy (testable)
// ---------------------------------------------------------------------------

/// Core deploy logic for Crimson Desert native mods. Pulled out of
/// [`CrimsonDesertNativePlugin::deploy_native`] so that the test module can
/// drive it against a tempdir install root.
///
/// Algorithm (spec §9 Step 4):
/// 1. Verify `install_root` exists.
/// 2. Reject sandboxed games (belt-and-suspenders).
/// 3. Take a native snapshot (best-effort; warn on fail).
/// 4. Back up `meta/0.papgt` to `meta/0.papgt.bak` (idempotent).
/// 5. For each enabled mod:
///    - Classify the staging dir.
///    - PreBuiltOverlay → copy paz+pamt into the next available `<NNNN>/`.
///    - LooseAssets / Empty → skip with a warn log.
///    - AsiDll → return Err (Windows-only artifact).
/// 6. Register each new group with [`PapgtEditor::register_group`] (currently
///    a Phase 1b stub that errs).
pub fn deploy_native_inner(
    detected: &DetectedGame,
    db: &Arc<ModDatabase>,
    install_root: &Path,
) -> Result<DeployResult, DeployerError> {
    // 1. Sandbox refusal (belt-and-suspenders).
    if let Some(native) = detected.runtime.native() {
        if native.sandboxed {
            return Err(DeployerError::Other(format!(
                "native modding refused for sandboxed app: {}",
                native.app_bundle_path.display()
            )));
        }
    }

    // 2. Install root must exist.
    if !install_root.exists() {
        return Err(DeployerError::Other(format!(
            "Crimson Desert install root does not exist: {}",
            install_root.display()
        )));
    }

    // 3. Pre-deploy snapshot (best-effort).
    if let Err(e) = crate::rollback::create_native_snapshot(
        db,
        &detected.game_id,
        "crimson-desert-deploy",
        &format!("Crimson Desert deploy to {}", install_root.display()),
    ) {
        log::warn!("snapshot before Crimson Desert deploy failed: {}", e);
    }

    // 4. PAPGT backup (idempotent).
    PapgtEditor::ensure_backup(install_root)?;

    // 5. Walk enabled mods.
    let enabled_mods = db
        .list_mods(&detected.game_id, CD_NATIVE_BOTTLE_SENTINEL)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let mut deployed_count = 0usize;
    let mut skipped_count = 0usize;
    let mut newly_registered_groups: Vec<u16> = Vec::new();

    for installed_mod in enabled_mods.iter().filter(|m| m.enabled) {
        let staging_dir = match &installed_mod.staging_path {
            Some(p) => PathBuf::from(p),
            None => {
                log::warn!(
                    "crimson_desert deploy_native: mod '{}' has no staging path, skipping",
                    installed_mod.name
                );
                skipped_count += 1;
                continue;
            }
        };
        if !staging_dir.exists() {
            log::warn!(
                "crimson_desert deploy_native: staging dir missing for mod '{}': {}",
                installed_mod.name,
                staging_dir.display()
            );
            skipped_count += 1;
            continue;
        }

        match classify_mod_staging(&staging_dir) {
            ModDeployKind::AsiDll(path) => {
                return Err(DeployerError::Other(format!(
                    "Windows-only mod artifact in mod '{}': {}. \
                     Crimson Desert macOS uses PAZ overlay groups (.paz + .pamt), \
                     not ASI/DLL plugins. Look for a macOS-native version of this mod.",
                    installed_mod.name,
                    path.display()
                )));
            }
            ModDeployKind::LooseAssets => {
                log::warn!(
                    "crimson_desert deploy_native: mod '{}' contains loose assets \
                     without a .paz/.pamt pair — skipping (Phase 1 deploys pre-built \
                     overlays only)",
                    installed_mod.name
                );
                skipped_count += 1;
            }
            ModDeployKind::Empty => {
                log::warn!(
                    "crimson_desert deploy_native: mod '{}' staging dir is empty, skipping",
                    installed_mod.name
                );
                skipped_count += 1;
            }
            ModDeployKind::PreBuiltOverlay { paz_and_pamt_pairs } => {
                // Assign the next overlay group for this mod's pair set. If a
                // mod ships more than one (NNNN/0.paz + NNNN/0.pamt) pair, each
                // pair gets its own freshly-assigned group number so we never
                // collide with the pair's original staging-side group naming.
                for (paz_src, pamt_src) in &paz_and_pamt_pairs {
                    let group = next_available_group_number(install_root);
                    let group_name = format_group_name(group);
                    let group_dir = install_root.join(&group_name);

                    // Path safety: the destination is always
                    // `<install_root>/<NNNN>/0.{paz,pamt}` — no traversal vector
                    // since group_name is a 4-digit decimal we built, but
                    // validate the destination relative names for defence in
                    // depth.
                    let paz_dest_name = paz_src
                        .file_name()
                        .and_then(|n| n.to_str())
                        .ok_or_else(|| {
                            DeployerError::Other("paz file name not UTF-8".into())
                        })?;
                    let pamt_dest_name = pamt_src
                        .file_name()
                        .and_then(|n| n.to_str())
                        .ok_or_else(|| {
                            DeployerError::Other("pamt file name not UTF-8".into())
                        })?;
                    if !is_safe_relative_path(paz_dest_name)
                        || !is_safe_relative_path(pamt_dest_name)
                    {
                        return Err(DeployerError::Other(format!(
                            "unsafe overlay file name in mod '{}'",
                            installed_mod.name
                        )));
                    }

                    std::fs::create_dir_all(&group_dir).map_err(|e| {
                        DeployerError::Other(format!("create overlay group dir: {}", e))
                    })?;

                    deploy_one_file(paz_src, &group_dir.join(paz_dest_name))?;
                    deploy_one_file(pamt_src, &group_dir.join(pamt_dest_name))?;
                    deployed_count += 2;
                    newly_registered_groups.push(group);
                }
            }
        }
    }

    // 6. Register each new group in meta/0.papgt. Phase 1b: this currently
    // returns Err — see PapgtEditor::register_group. We attempt registration
    // only if at least one group was actually copied; a no-op deploy succeeds.
    for group in &newly_registered_groups {
        PapgtEditor::register_group(install_root, *group)?;
    }

    Ok(DeployResult {
        deployed_count,
        skipped_count,
        fallback_used: false,
    })
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register() {
    crate::games::register_plugin(Arc::new(CrimsonDesertNativePlugin));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_scanner::NativeAppCandidate;
    use crate::plist::InfoPlist;
    use crate::runtime::{Architecture, GameRuntime, NativeContext, NativeSource};
    use std::path::PathBuf;

    // ── Test helpers ────────────────────────────────────────────────────────

    /// Build a synthetic `NativeAppCandidate` for use in unit tests.
    ///
    /// The detection logic reads `bundle_identifier` and `bundle_executable`
    /// from `info`, and `sandboxed` / `source` / `architecture` from the
    /// candidate directly. The remaining `InfoPlist` fields are unused in
    /// detection and are given zero-value stubs here.
    fn fake_candidate(
        bundle_path: &str,
        bundle_id: &str,
        exe_name: &str,
        sandboxed: bool,
        source: NativeSource,
        arch: Architecture,
    ) -> NativeAppCandidate {
        NativeAppCandidate {
            bundle_path: PathBuf::from(bundle_path),
            info: InfoPlist {
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

    /// Build a synthetic `DetectedGame` with a (non-sandboxed) native runtime.
    fn fake_detected(game_path: &str) -> DetectedGame {
        fake_detected_at(Path::new(game_path))
    }

    /// Build a synthetic `DetectedGame` rooted at an arbitrary path. Used by
    /// tempdir tests that need a real, existing install root.
    fn fake_detected_at(install_root: &Path) -> DetectedGame {
        DetectedGame {
            game_id: "crimson_desert_native".into(),
            display_name: "Crimson Desert".into(),
            nexus_slug: "crimsondesert".into(),
            game_path: install_root.to_path_buf(),
            exe_path: None,
            data_dir: install_root.join("Paz"),
            runtime: GameRuntime::Native(NativeContext {
                app_bundle_path: install_root.join("Crimson Desert.app"),
                game_data_root: install_root.to_path_buf(),
                architecture: Architecture::AppleSilicon,
                sandboxed: false,
                source: NativeSource::Steam,
            }),
            steam_app_id: Some(CD_STEAM_APP_ID.into()),
        }
    }

    /// Create vanilla overlay group directories `0000`…`max - 1` under `root`.
    fn populate_vanilla_groups(root: &Path, max_exclusive: u16) {
        std::fs::create_dir_all(root).unwrap();
        for n in 0..max_exclusive {
            std::fs::create_dir_all(root.join(format_group_name(n))).unwrap();
        }
    }

    /// Insert a single enabled mod into `db` with `staging_path` populated and
    /// return its database id.
    fn insert_mod_with_staging(db: &Arc<ModDatabase>, name: &str, staging: &Path) -> i64 {
        let mod_id = db
            .add_mod(
                "crimson_desert_native",
                "",
                None,
                name,
                "1.0.0",
                &format!("{}.zip", name),
                &[],
            )
            .unwrap();
        db.set_staging_path(mod_id, &staging.to_string_lossy())
            .unwrap();
        mod_id
    }

    /// Activate the gated deploy pipeline within a single test. The const can
    /// only be read, not mutated, so tests that need to drive
    /// `deploy_native_inner` call it directly — this helper just documents the
    /// intent at the call site.
    fn assert_verified_is_false_by_default() {
        assert!(
            !VERIFIED,
            "VERIFIED const must be false by default so prod stays gated"
        );
    }

    // ── Metadata ─────────────────────────────────────────────────────────────

    #[test]
    fn crimson_desert_plugin_metadata() {
        let p = CrimsonDesertNativePlugin;
        assert_eq!(p.game_id(), "crimson_desert_native");
        assert_eq!(p.display_name(), "Crimson Desert (Native)");
        assert_eq!(p.nexus_slug(), "crimsondesert");
        assert!(!p.executables().is_empty());
    }

    // ── Detection filter: bundle identifier ──────────────────────────────────

    #[test]
    fn detect_native_filters_by_bundle_id() {
        let candidates = vec![
            fake_candidate(
                "/Applications/Crimson Desert.app",
                "com.pearlabyss.CrimsonDesert",
                "Crimson Desert",
                false,
                NativeSource::Steam,
                Architecture::AppleSilicon,
            ),
            fake_candidate(
                "/Applications/Other.app",
                "com.other.app",
                "Other",
                false,
                NativeSource::SystemApplications,
                Architecture::AppleSilicon,
            ),
        ];
        let result = detect_from_candidates(candidates);
        assert_eq!(result.len(), 1, "only the CD candidate should match");
        assert_eq!(result[0].game_id, "crimson_desert_native");
    }

    // ── Detection filter: Intel-only skipped ────────────────────────────────

    #[test]
    fn detect_native_skips_intel_only_candidates() {
        let candidates = vec![fake_candidate(
            "/Applications/Crimson Desert.app",
            "com.pearlabyss.CrimsonDesert",
            "Crimson Desert",
            false,
            NativeSource::Steam,
            Architecture::IntelOnly,
        )];
        let result = detect_from_candidates(candidates);
        assert_eq!(
            result.len(),
            0,
            "Intel-only Crimson Desert should not match — no Intel build exists"
        );
    }

    // ── Detection filter: sandboxed skipped ─────────────────────────────────

    #[test]
    fn detect_native_skips_sandboxed() {
        let candidates = vec![fake_candidate(
            "/Applications/Crimson Desert.app",
            "com.pearlabyss.CrimsonDesert",
            "Crimson Desert",
            true, // sandboxed = App Store version
            NativeSource::AppStore,
            Architecture::AppleSilicon,
        )];
        let result = detect_from_candidates(candidates);
        assert_eq!(result.len(), 0, "sandboxed App Store version must be filtered");
    }

    // ── Detection: Steam app ID propagation ─────────────────────────────────

    #[test]
    fn detect_native_populates_steam_app_id_for_steam_source() {
        let candidates = vec![fake_candidate(
            "/Applications/Crimson Desert.app",
            "com.pearlabyss.CrimsonDesert",
            "Crimson Desert",
            false,
            NativeSource::Steam,
            Architecture::AppleSilicon,
        )];
        let result = detect_from_candidates(candidates);
        assert_eq!(
            result[0].steam_app_id.as_deref(),
            Some("3321460"),
            "Steam source must set steam_app_id"
        );
    }

    #[test]
    fn detect_native_omits_steam_app_id_for_manual_source() {
        let candidates = vec![fake_candidate(
            "/Applications/Crimson Desert.app",
            "com.pearlabyss.CrimsonDesert",
            "Crimson Desert",
            false,
            NativeSource::Manual,
            Architecture::AppleSilicon,
        )];
        let result = detect_from_candidates(candidates);
        assert_eq!(
            result[0].steam_app_id, None,
            "Manual source must not set steam_app_id"
        );
    }

    // ── Detection: executable-name fallback ────────────────────────────────

    #[test]
    fn detect_native_falls_back_to_executable_name() {
        // Unrecognised bundle ID, but bundle_executable matches CD_BUNDLE_EXECUTABLE.
        let candidates = vec![fake_candidate(
            "/Applications/CD.app",
            "com.unknown.foo",
            "Crimson Desert",
            false,
            NativeSource::Steam,
            Architecture::AppleSilicon,
        )];
        let result = detect_from_candidates(candidates);
        assert_eq!(result.len(), 1, "executable-name fallback should produce a match");
    }

    // ── Data dir ─────────────────────────────────────────────────────────────

    #[test]
    fn data_dir_is_paz_subdir_of_install_root() {
        let p = CrimsonDesertNativePlugin;
        let install_root = PathBuf::from(
            "/Users/x/Library/Application Support/Steam/steamapps/common/Crimson Desert",
        );
        assert_eq!(
            p.get_data_dir(&install_root),
            install_root.join("Paz"),
            "data_dir must be <install_root>/Paz"
        );
    }

    // ── Deploy blocked ────────────────────────────────────────────────────────

    #[test]
    fn deploy_native_returns_blocked_error_until_path_verified() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(
            crate::database::ModDatabase::new(&tmp.path().join("test.db"))
                .expect("test db"),
        );
        let detected = fake_detected("/fake/install");
        let result = CrimsonDesertNativePlugin.deploy_native(&detected, &db);
        assert!(result.is_err(), "deploy_native must return Err when deploy is blocked");
        let err = format!("{:?}", result.unwrap_err());
        assert!(
            err.to_lowercase().contains("blocked")
                || err.to_lowercase().contains("unverified")
                || err.to_lowercase().contains("paz"),
            "expected a BLOCKED/unverified/PAZ error message, got: {}",
            err
        );
    }

    #[test]
    fn deploy_native_refuses_sandboxed_game() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(
            crate::database::ModDatabase::new(&tmp.path().join("test.db"))
                .expect("test db"),
        );
        let detected = DetectedGame {
            game_id: "crimson_desert_native".into(),
            display_name: "Crimson Desert".into(),
            nexus_slug: "crimsondesert".into(),
            game_path: PathBuf::from("/fake"),
            exe_path: None,
            data_dir: PathBuf::from("/fake/Paz"),
            runtime: GameRuntime::Native(NativeContext {
                app_bundle_path: PathBuf::from(
                    "/Library/Containers/com.pearlabyss.CrimsonDesert/Crimson Desert.app",
                ),
                game_data_root: PathBuf::from("/fake"),
                architecture: Architecture::AppleSilicon,
                sandboxed: true, // App Store sandboxed
                source: NativeSource::AppStore,
            }),
            steam_app_id: None,
        };
        let result = CrimsonDesertNativePlugin.deploy_native(&detected, &db);
        assert!(result.is_err(), "deploy_native must refuse sandboxed game");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("sandboxed"),
            "error must mention 'sandboxed': {msg}"
        );
    }

    #[test]
    fn deploy_native_refuses_intel_only_architecture() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(
            crate::database::ModDatabase::new(&tmp.path().join("test.db"))
                .expect("test db"),
        );
        let detected = DetectedGame {
            game_id: "crimson_desert_native".into(),
            display_name: "Crimson Desert".into(),
            nexus_slug: "crimsondesert".into(),
            game_path: PathBuf::from("/fake"),
            exe_path: None,
            data_dir: PathBuf::from("/fake/Paz"),
            runtime: GameRuntime::Native(NativeContext {
                app_bundle_path: PathBuf::from("/Applications/Crimson Desert.app"),
                game_data_root: PathBuf::from("/fake"),
                architecture: Architecture::IntelOnly, // should not exist
                sandboxed: false,
                source: NativeSource::Steam,
            }),
            steam_app_id: Some(CD_STEAM_APP_ID.into()),
        };
        let result = CrimsonDesertNativePlugin.deploy_native(&detected, &db);
        assert!(result.is_err(), "deploy_native must refuse Intel-only architecture");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.to_lowercase().contains("intel") || msg.to_lowercase().contains("silicon"),
            "error must mention Intel/Silicon architecture: {msg}"
        );
    }

    // ── Group number assignment ──────────────────────────────────────────────

    #[test]
    fn next_available_group_number_returns_36_when_install_has_only_vanilla() {
        let tmp = tempfile::tempdir().unwrap();
        populate_vanilla_groups(tmp.path(), 36);
        assert_eq!(next_available_group_number(tmp.path()), 36);
    }

    #[test]
    fn next_available_group_number_skips_nonnumeric_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        populate_vanilla_groups(tmp.path(), 36);
        // Add noise that PAPGT's loader would also ignore.
        std::fs::create_dir_all(tmp.path().join("Paz")).unwrap();
        std::fs::create_dir_all(tmp.path().join("meta")).unwrap();
        std::fs::create_dir_all(tmp.path().join("misc-mod")).unwrap();
        // A 3-digit dir is not a valid overlay group either.
        std::fs::create_dir_all(tmp.path().join("099")).unwrap();
        std::fs::write(tmp.path().join("0037"), b"not a dir").unwrap();
        assert_eq!(next_available_group_number(tmp.path()), 36);
    }

    #[test]
    fn next_available_group_number_finds_gap_above_vanilla() {
        let tmp = tempfile::tempdir().unwrap();
        populate_vanilla_groups(tmp.path(), 36);
        std::fs::create_dir_all(tmp.path().join("0040")).unwrap();
        // Max+1 semantics, NOT gap fill — see helper doc.
        assert_eq!(next_available_group_number(tmp.path()), 41);
    }

    // ── classify_mod_staging ─────────────────────────────────────────────────

    #[test]
    fn classify_mod_staging_detects_paz_pamt_pair() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("0.paz"), b"paz bytes").unwrap();
        std::fs::write(tmp.path().join("0.pamt"), b"pamt bytes").unwrap();
        match classify_mod_staging(tmp.path()) {
            ModDeployKind::PreBuiltOverlay { paz_and_pamt_pairs } => {
                assert_eq!(paz_and_pamt_pairs.len(), 1);
            }
            other => panic!("expected PreBuiltOverlay, got {:?}", other),
        }
    }

    #[test]
    fn classify_mod_staging_rejects_asi() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("inject.asi"), b"asi").unwrap();
        match classify_mod_staging(tmp.path()) {
            ModDeployKind::AsiDll(_) => {}
            other => panic!("expected AsiDll, got {:?}", other),
        }
    }

    #[test]
    fn classify_mod_staging_returns_empty_when_no_known_kind() {
        // Spec semantics: a staging dir with only loose, non-overlay assets
        // becomes LooseAssets (deferred to Phase 1+). A truly empty dir is
        // ModDeployKind::Empty.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("readme.txt"), b"hi").unwrap();
        assert_eq!(classify_mod_staging(tmp.path()), ModDeployKind::LooseAssets);

        let empty = tempfile::tempdir().unwrap();
        assert_eq!(classify_mod_staging(empty.path()), ModDeployKind::Empty);
    }

    // ── deploy_native_inner: preconditions ──────────────────────────────────

    #[test]
    fn deploy_native_inner_refuses_when_install_root_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(ModDatabase::new(&tmp.path().join("test.db")).unwrap());
        let missing = tmp.path().join("does-not-exist");
        let detected = fake_detected_at(&missing);
        let err = deploy_native_inner(&detected, &db, &missing).unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("does not exist"),
            "error must say install root missing: {msg}"
        );
    }

    #[test]
    fn deploy_native_inner_refuses_sandboxed_game() {
        let tmp = tempfile::tempdir().unwrap();
        let install_root = tmp.path().join("install");
        std::fs::create_dir_all(&install_root).unwrap();
        let db = Arc::new(ModDatabase::new(&tmp.path().join("test.db")).unwrap());
        let detected = DetectedGame {
            game_id: "crimson_desert_native".into(),
            display_name: "Crimson Desert".into(),
            nexus_slug: "crimsondesert".into(),
            game_path: install_root.clone(),
            exe_path: None,
            data_dir: install_root.join("Paz"),
            runtime: GameRuntime::Native(NativeContext {
                app_bundle_path: install_root.join("Crimson Desert.app"),
                game_data_root: install_root.clone(),
                architecture: Architecture::AppleSilicon,
                sandboxed: true,
                source: NativeSource::AppStore,
            }),
            steam_app_id: None,
        };
        let err = deploy_native_inner(&detected, &db, &install_root).unwrap_err();
        assert!(format!("{}", err).contains("sandboxed"));
    }

    // ── deploy_native_inner: PAPGT backup ───────────────────────────────────

    #[test]
    fn deploy_native_inner_creates_papgt_backup_when_none_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let install_root = tmp.path().to_path_buf();
        populate_vanilla_groups(&install_root, 36);
        std::fs::create_dir_all(install_root.join("meta")).unwrap();
        std::fs::write(install_root.join("meta/0.papgt"), b"vanilla-papgt").unwrap();

        let db = Arc::new(ModDatabase::new(&tmp.path().join("test.db")).unwrap());

        // Stage a paz+pamt mod (Phase 1b register_group will Err — that's fine,
        // we only need to assert that the backup was created BEFORE the error).
        let staging = tmp.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("0.paz"), b"mod-paz").unwrap();
        std::fs::write(staging.join("0.pamt"), b"mod-pamt").unwrap();
        insert_mod_with_staging(&db, "TestMod", &staging);

        let detected = fake_detected_at(&install_root);
        let _ = deploy_native_inner(&detected, &db, &install_root);

        let backup = install_root.join("meta/0.papgt.bak");
        assert!(backup.exists(), "0.papgt.bak must be created");
        let bytes = std::fs::read(&backup).unwrap();
        assert_eq!(bytes, b"vanilla-papgt");
    }

    #[test]
    fn deploy_native_inner_skips_papgt_backup_when_bak_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let install_root = tmp.path().to_path_buf();
        std::fs::create_dir_all(install_root.join("meta")).unwrap();
        std::fs::write(install_root.join("meta/0.papgt"), b"current-papgt").unwrap();
        std::fs::write(install_root.join("meta/0.papgt.bak"), b"original-vanilla").unwrap();

        let db = Arc::new(ModDatabase::new(&tmp.path().join("test.db")).unwrap());
        let detected = fake_detected_at(&install_root);
        let _ = deploy_native_inner(&detected, &db, &install_root);

        let bytes = std::fs::read(install_root.join("meta/0.papgt.bak")).unwrap();
        assert_eq!(
            bytes, b"original-vanilla",
            "existing backup must be preserved byte-for-byte"
        );
    }

    // ── deploy_native_inner: overlay copy ───────────────────────────────────
    //
    // PAPGT registration is a Phase 1b stub that Errs. To assert the overlay
    // copy happened, these tests inspect the filesystem AFTER catching that
    // error — the copy executes before register_group is called.

    #[test]
    fn deploy_native_inner_assigns_0036_to_first_mod() {
        let tmp = tempfile::tempdir().unwrap();
        let install_root = tmp.path().to_path_buf();
        populate_vanilla_groups(&install_root, 36);

        let db = Arc::new(ModDatabase::new(&tmp.path().join("test.db")).unwrap());

        let staging = tmp.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("0.paz"), b"mod-paz").unwrap();
        std::fs::write(staging.join("0.pamt"), b"mod-pamt").unwrap();
        insert_mod_with_staging(&db, "TestMod", &staging);

        let detected = fake_detected_at(&install_root);
        let _ = deploy_native_inner(&detected, &db, &install_root);

        let group_dir = install_root.join("0036");
        assert!(group_dir.exists(), "0036 must be created");
        assert!(group_dir.join("0.paz").exists());
        assert!(group_dir.join("0.pamt").exists());
        let paz_bytes = std::fs::read(group_dir.join("0.paz")).unwrap();
        assert_eq!(paz_bytes, b"mod-paz");
    }

    #[test]
    fn deploy_native_inner_assigns_sequential_groups_for_multiple_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let install_root = tmp.path().to_path_buf();
        populate_vanilla_groups(&install_root, 36);

        let db = Arc::new(ModDatabase::new(&tmp.path().join("test.db")).unwrap());

        for i in 0..3 {
            let staging = tmp.path().join(format!("staging-{}", i));
            std::fs::create_dir_all(&staging).unwrap();
            std::fs::write(staging.join("0.paz"), format!("mod-paz-{}", i)).unwrap();
            std::fs::write(staging.join("0.pamt"), format!("mod-pamt-{}", i)).unwrap();
            insert_mod_with_staging(&db, &format!("Mod{}", i), &staging);
        }

        let detected = fake_detected_at(&install_root);
        let _ = deploy_native_inner(&detected, &db, &install_root);

        for n in 36..=38u16 {
            let group_dir = install_root.join(format_group_name(n));
            assert!(
                group_dir.join("0.paz").exists(),
                "expected group {} to exist with 0.paz",
                n
            );
            assert!(group_dir.join("0.pamt").exists());
        }
    }

    #[test]
    fn deploy_native_inner_returns_err_for_asi_mod() {
        let tmp = tempfile::tempdir().unwrap();
        let install_root = tmp.path().to_path_buf();
        populate_vanilla_groups(&install_root, 36);

        let db = Arc::new(ModDatabase::new(&tmp.path().join("test.db")).unwrap());

        let staging = tmp.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("hook.asi"), b"asi bytes").unwrap();
        insert_mod_with_staging(&db, "AsiMod", &staging);

        let detected = fake_detected_at(&install_root);
        let err = deploy_native_inner(&detected, &db, &install_root).unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("Windows-only"),
            "ASI/DLL must be rejected with Windows-only message: {msg}"
        );
    }

    #[test]
    fn deploy_native_inner_takes_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let install_root = tmp.path().to_path_buf();
        populate_vanilla_groups(&install_root, 36);

        let db = Arc::new(ModDatabase::new(&tmp.path().join("test.db")).unwrap());
        crate::rollback::init_schema(&db).unwrap();

        let detected = fake_detected_at(&install_root);
        // No mods registered — deploy is a no-op but must still snapshot.
        deploy_native_inner(&detected, &db, &install_root).unwrap();

        let snapshots =
            crate::rollback::list_snapshots(&db, "crimson_desert_native", "").unwrap();
        assert!(!snapshots.is_empty(), "deploy must create at least one snapshot");
        assert_eq!(snapshots[0].name, "crimson-desert-deploy");
    }

    // ── Verified gate ───────────────────────────────────────────────────────

    #[test]
    fn deploy_native_returns_blocked_when_verified_is_false() {
        assert_verified_is_false_by_default();
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(ModDatabase::new(&tmp.path().join("test.db")).unwrap());
        let detected = fake_detected("/fake/install");
        let err = CrimsonDesertNativePlugin
            .deploy_native(&detected, &db)
            .unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("VERIFIED") || msg.contains("gated"),
            "expected gated/VERIFIED message, got: {msg}"
        );
    }
}
