//! Script extenders for games that Steam (or GameHub) must start itself.
//!
//! Steam Play prefixes (Proton on Linux, NotProton on macOS) and GameHub
//! containers can't run a script extender's loader or a modding tool
//! directly: the game has to be started by the Steam client (or GameHub) to
//! get its environment, and they start the game's launcher exe. So Corkscrew
//! puts its launch trampoline (`resources/trampoline/`) in the launcher's
//! place and keeps the real launcher as `<launcher>.corkscrew-orig`.
//!
//! The trampoline runs, in order: a one-shot `corkscrew_launch.txt` request
//! (a tool, see [`run_once`]), the program in `corkscrew_default.txt` (the
//! script extender after [`apply`]), or the real launcher.
//!
//! Steam's "Verify integrity" and game updates restore the real launcher, so
//! [`apply`] / [`run_once`] check it before every launch; [`restore`] puts
//! the real launcher back.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use log::info;

/// Suffix of the kept original launcher.
pub const ORIGINAL_SUFFIX: &str = ".corkscrew-orig";

/// The launch trampoline (built by `scripts/build-trampoline.sh` from
/// `resources/trampoline/trampoline.c`).
pub const TRAMPOLINE: &[u8] = include_bytes!("../resources/trampoline/corkscrew-trampoline.exe");

/// One-shot request the trampoline reads and deletes.
pub const REQUEST_FILE: &str = "corkscrew_launch.txt";
/// What the trampoline runs when there's no request.
pub const DEFAULT_FILE: &str = "corkscrew_default.txt";
/// What the trampoline ran (written next to it, for troubleshooting).
pub const LOG_FILE: &str = "corkscrew_trampoline.log";

/// A game's launcher exe and the script-extender loader that replaces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LauncherSwap {
    pub launcher: &'static str,
    pub loader: &'static str,
}

/// The launcher swap for a game, if Corkscrew knows its launcher.
pub fn for_game(game_id: &str) -> Option<LauncherSwap> {
    let launcher = match game_id {
        "skyrimse" => "SkyrimSELauncher.exe",
        "skyrim" => "SkyrimLauncher.exe",
        "fallout4" => "Fallout4Launcher.exe",
        "falloutnv" => "FalloutNVLauncher.exe",
        "fallout3" => "FalloutLauncher.exe",
        "oblivion" => "OblivionLauncher.exe",
        _ => return None,
    };
    let loader = crate::games::script_extender_loader(game_id)?;
    Some(LauncherSwap { launcher, loader })
}

/// Case-insensitive lookup of a file in the game folder.
fn find(game_path: &Path, name: &str) -> Option<PathBuf> {
    let exact = game_path.join(name);
    if fs::symlink_metadata(&exact).is_ok() {
        return Some(exact);
    }
    fs::read_dir(game_path).ok()?.flatten().find_map(|e| {
        e.file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
            .then(|| e.path())
    })
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

fn refuse_symlink(path: &Path) -> io::Result<()> {
    if is_symlink(path) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to replace symlink {}", path.display()),
        ));
    }
    Ok(())
}

fn same_contents(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(ma), Ok(mb)) if ma.len() == mb.len() => {
            matches!((fs::read(a), fs::read(b)), (Ok(x), Ok(y)) if x == y)
        }
        _ => false,
    }
}

fn launcher_paths(game_path: &Path, swap: LauncherSwap) -> (PathBuf, PathBuf) {
    let launcher = find(game_path, swap.launcher).unwrap_or_else(|| game_path.join(swap.launcher));
    let original = PathBuf::from(format!("{}{ORIGINAL_SUFFIX}", launcher.display()));
    (launcher, original)
}

fn is_trampoline(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.len() == TRAMPOLINE.len() as u64)
        && fs::read(path).is_ok_and(|b| b == TRAMPOLINE)
}

/// Whether `path` is a copy of the loader (the swap older builds made).
fn is_loader_copy(game_path: &Path, swap: LauncherSwap, path: &Path) -> bool {
    find(game_path, swap.loader).is_some_and(|loader| same_contents(&loader, path))
}

/// Whether the trampoline is in the launcher's place.
pub fn is_installed(game_path: &Path, swap: LauncherSwap) -> bool {
    is_trampoline(&launcher_paths(game_path, swap).0)
}

/// Whether a plain Play currently starts the script extender.
pub fn is_applied(game_path: &Path, swap: LauncherSwap) -> bool {
    is_installed(game_path, swap)
        && fs::read_to_string(game_path.join(DEFAULT_FILE))
            .is_ok_and(|d| d.lines().next().is_some_and(|l| l.trim().eq_ignore_ascii_case(swap.loader)))
}

/// Put the trampoline in the launcher's place, keeping the real launcher.
/// Returns whether anything changed.
pub fn install(game_path: &Path, swap: LauncherSwap) -> io::Result<bool> {
    let (launcher, original) = launcher_paths(game_path, swap);
    refuse_symlink(&launcher)?;
    refuse_symlink(&original)?;
    if is_trampoline(&launcher) {
        return Ok(false);
    }
    if launcher.exists() {
        if original.exists() && is_loader_copy(game_path, swap, &launcher) {
            // The old loader-copy swap; the real launcher is already kept.
            fs::remove_file(&launcher)?;
        } else {
            // A real launcher (first install, or Steam restored it after a
            // verify/update): it becomes the kept original.
            fs::rename(&launcher, &original)?;
        }
    }
    fs::write(&launcher, TRAMPOLINE)?;
    info!(
        "Launch trampoline in place of {} (original kept as {})",
        swap.launcher,
        original.display()
    );
    Ok(true)
}

fn write_request_file(path: &Path, program: &str, args: &str, workdir: Option<&str>) -> io::Result<()> {
    for field in [program, args, workdir.unwrap_or("")] {
        if field.contains(['\n', '\r', '\0']) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "launch request fields must be one line"));
        }
    }
    refuse_symlink(path)?;
    fs::write(path, format!("{program}\n{args}\n{}\n", workdir.unwrap_or("")))
}

/// Make a plain Play start the script extender's loader. Errors if the
/// loader isn't installed. Returns whether anything changed.
pub fn apply(game_path: &Path, swap: LauncherSwap) -> io::Result<bool> {
    let loader = find(game_path, swap.loader).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} isn't installed in {}", swap.loader, game_path.display()),
        )
    })?;
    refuse_symlink(&loader)?;
    if is_applied(game_path, swap) {
        return Ok(false);
    }
    install(game_path, swap)?;
    write_request_file(&game_path.join(DEFAULT_FILE), swap.loader, "", None)?;
    info!("Script extender: {} now starts {}", swap.launcher, swap.loader);
    Ok(true)
}

/// Run `program` once inside the game's environment at the next launch
/// (the trampoline reads and deletes the request). `program` is relative to
/// the game folder or an absolute Windows path; see [`windows_path_for`].
pub fn run_once(
    game_path: &Path,
    swap: LauncherSwap,
    program: &str,
    args: &str,
    workdir: Option<&str>,
) -> io::Result<()> {
    install(game_path, swap)?;
    write_request_file(&game_path.join(REQUEST_FILE), program, args, workdir)
}

/// How the trampoline (running in `game_path`) should name `host_path`:
/// relative when it's inside the game folder, else via Wine's `Z:` drive
/// (the host root in Proton, NotProton and GameHub prefixes).
pub fn windows_path_for(game_path: &Path, host_path: &Path) -> String {
    let game = game_path.canonicalize().unwrap_or_else(|_| game_path.to_path_buf());
    let host = host_path.canonicalize().unwrap_or_else(|_| host_path.to_path_buf());
    match host.strip_prefix(&game) {
        Ok(rel) => rel.to_string_lossy().replace('/', "\\"),
        Err(_) => format!("Z:{}", host.to_string_lossy().replace('/', "\\")),
    }
}

/// Put the real launcher back and remove the trampoline's files. Returns
/// whether anything changed.
pub fn restore(game_path: &Path, swap: LauncherSwap) -> io::Result<bool> {
    let (launcher, original) = launcher_paths(game_path, swap);
    refuse_symlink(&launcher)?;
    refuse_symlink(&original)?;
    let mut changed = false;
    for name in [DEFAULT_FILE, REQUEST_FILE, LOG_FILE] {
        let p = game_path.join(name);
        if fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_file()) {
            fs::remove_file(&p)?;
            changed = true;
        }
    }
    if !original.is_file() {
        return Ok(changed);
    }
    let ours = !launcher.exists()
        || is_trampoline(&launcher)
        || is_loader_copy(game_path, swap, &launcher)
        // Loader since uninstalled: anything that isn't the original is ours.
        || (find(game_path, swap.loader).is_none() && !same_contents(&launcher, &original));
    if !ours {
        // Steam already put a (possibly newer) real launcher back.
        fs::remove_file(&original)?;
        return Ok(true);
    }
    if launcher.exists() {
        fs::remove_file(&launcher)?;
    }
    fs::rename(&original, &launcher)?;
    info!("Restored the original {}", swap.launcher);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SWAP: LauncherSwap = LauncherSwap {
        launcher: "SkyrimSELauncher.exe",
        loader: "skse64_loader.exe",
    };

    fn read(p: &Path) -> String {
        fs::read_to_string(p).unwrap()
    }

    fn game() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("SkyrimSELauncher.exe"), "launcher v1").unwrap();
        fs::write(tmp.path().join("skse64_loader.exe"), "skse loader").unwrap();
        tmp
    }

    #[test]
    fn trampoline_is_a_small_windows_exe() {
        assert_eq!(&TRAMPOLINE[..2], b"MZ");
        assert!(TRAMPOLINE.len() < 64 * 1024);
    }

    #[test]
    fn skyrim_and_fallout4_have_swaps() {
        assert_eq!(for_game("skyrimse"), Some(SWAP));
        assert_eq!(for_game("skyrim").map(|s| s.launcher), Some("SkyrimLauncher.exe"));
        assert_eq!(for_game("fallout4").map(|s| s.loader), Some("f4se_loader.exe"));
        assert_eq!(for_game("hogwartslegacy"), None);
    }

    #[test]
    fn apply_then_restore_round_trips() {
        let tmp = game();
        let g = tmp.path();
        assert!(apply(g, SWAP).unwrap());
        assert!(is_trampoline(&g.join("SkyrimSELauncher.exe")));
        assert_eq!(read(&g.join("SkyrimSELauncher.exe.corkscrew-orig")), "launcher v1");
        assert_eq!(read(&g.join(DEFAULT_FILE)), "skse64_loader.exe\n\n\n");
        assert!(is_applied(g, SWAP));
        assert!(!apply(g, SWAP).unwrap(), "second apply is a no-op");

        assert!(restore(g, SWAP).unwrap());
        assert_eq!(read(&g.join("SkyrimSELauncher.exe")), "launcher v1");
        assert!(!g.join("SkyrimSELauncher.exe.corkscrew-orig").exists());
        assert!(!g.join(DEFAULT_FILE).exists());
        assert!(!restore(g, SWAP).unwrap(), "second restore is a no-op");
    }

    #[test]
    fn run_once_writes_a_one_shot_request_without_changing_the_default() {
        let tmp = game();
        let g = tmp.path();
        run_once(g, SWAP, "Tools\\SSEEdit\\SSEEdit.exe", "-quickautoclean", Some("Tools\\SSEEdit")).unwrap();
        assert!(is_installed(g, SWAP));
        assert!(!is_applied(g, SWAP), "plain Play still starts the real launcher");
        assert_eq!(
            read(&g.join(REQUEST_FILE)),
            "Tools\\SSEEdit\\SSEEdit.exe\n-quickautoclean\nTools\\SSEEdit\n"
        );
        assert!(run_once(g, SWAP, "a.exe", "x\ny", None).is_err(), "no injected lines");
    }

    #[test]
    fn steam_verify_restoring_a_newer_launcher_is_kept() {
        let tmp = game();
        let g = tmp.path();
        apply(g, SWAP).unwrap();
        // Steam "Verify files" puts a (newer) real launcher back.
        fs::write(g.join("SkyrimSELauncher.exe"), "launcher v2").unwrap();
        assert!(!is_installed(g, SWAP));
        assert!(apply(g, SWAP).unwrap(), "re-applied before the next launch");
        assert_eq!(read(&g.join("SkyrimSELauncher.exe.corkscrew-orig")), "launcher v2");

        // Restoring after another verify keeps Steam's launcher.
        fs::write(g.join("SkyrimSELauncher.exe"), "launcher v3").unwrap();
        restore(g, SWAP).unwrap();
        assert_eq!(read(&g.join("SkyrimSELauncher.exe")), "launcher v3");
        assert!(!g.join("SkyrimSELauncher.exe.corkscrew-orig").exists());
    }

    #[test]
    fn migrates_the_old_loader_copy_swap() {
        let tmp = game();
        let g = tmp.path();
        // What earlier builds left: loader copied over the launcher.
        fs::rename(g.join("SkyrimSELauncher.exe"), g.join("SkyrimSELauncher.exe.corkscrew-orig")).unwrap();
        fs::copy(g.join("skse64_loader.exe"), g.join("SkyrimSELauncher.exe")).unwrap();
        apply(g, SWAP).unwrap();
        assert!(is_trampoline(&g.join("SkyrimSELauncher.exe")));
        assert_eq!(read(&g.join("SkyrimSELauncher.exe.corkscrew-orig")), "launcher v1");
        restore(g, SWAP).unwrap();
        assert_eq!(read(&g.join("SkyrimSELauncher.exe")), "launcher v1");
    }

    #[test]
    fn restore_after_loader_uninstalled_brings_the_launcher_back() {
        let tmp = game();
        let g = tmp.path();
        apply(g, SWAP).unwrap();
        fs::remove_file(g.join("skse64_loader.exe")).unwrap();
        restore(g, SWAP).unwrap();
        assert_eq!(read(&g.join("SkyrimSELauncher.exe")), "launcher v1");
    }

    #[test]
    fn apply_without_loader_fails_and_changes_nothing() {
        let tmp = game();
        let g = tmp.path();
        fs::remove_file(g.join("skse64_loader.exe")).unwrap();
        assert!(apply(g, SWAP).is_err());
        assert_eq!(read(&g.join("SkyrimSELauncher.exe")), "launcher v1");
    }

    #[test]
    fn windows_paths_are_relative_inside_the_game_and_z_outside() {
        let tmp = game();
        let g = tmp.path();
        fs::create_dir_all(g.join("Tools/xEdit")).unwrap();
        fs::write(g.join("Tools/xEdit/SSEEdit.exe"), "x").unwrap();
        assert_eq!(windows_path_for(g, &g.join("Tools/xEdit/SSEEdit.exe")), "Tools\\xEdit\\SSEEdit.exe");
        let other = tempfile::tempdir().unwrap();
        let w = windows_path_for(g, other.path());
        assert!(w.starts_with("Z:\\") && !w.contains('/'), "{w}");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_launcher_is_never_replaced() {
        let tmp = game();
        let g = tmp.path();
        let outside = g.join("outside.exe");
        fs::write(&outside, "elsewhere").unwrap();
        fs::remove_file(g.join("SkyrimSELauncher.exe")).unwrap();
        std::os::unix::fs::symlink(&outside, g.join("SkyrimSELauncher.exe")).unwrap();
        assert!(apply(g, SWAP).is_err());
        assert_eq!(read(&outside), "elsewhere");
    }
}

#[cfg(test)]
mod live_tests {
    /// Live launcher swap on a real install (changes game files):
    /// `LIVE_SWAP_GAME=<game dir> LIVE_SWAP_ACTION=apply|restore
    ///  cargo test --lib live_launcher_swap -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_launcher_swap() {
        let game = std::env::var("LIVE_SWAP_GAME").expect("LIVE_SWAP_GAME");
        let action = std::env::var("LIVE_SWAP_ACTION").expect("LIVE_SWAP_ACTION");
        let swap = super::for_game("skyrimse").unwrap();
        let path = std::path::Path::new(&game);
        let changed = match action.as_str() {
            "apply" => super::apply(path, swap).unwrap(),
            "restore" => super::restore(path, swap).unwrap(),
            other => panic!("unknown action {other}"),
        };
        eprintln!("{action}: changed={changed} applied_now={}", super::is_applied(path, swap));
    }
}
