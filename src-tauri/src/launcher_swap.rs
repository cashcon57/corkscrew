//! Script extenders for games that Steam (or GameHub) must start itself.
//!
//! Steam Play prefixes (Proton on Linux, NotProton on macOS) and GameHub
//! containers can't run a script extender's loader directly: the game has to
//! be started by the Steam client (or GameHub) to get its Steam bridge, and
//! Steam starts the game's launcher exe. So, like the usual Proton modding
//! setups, Corkscrew puts the loader in the launcher's place:
//! `SkyrimSELauncher.exe` becomes a copy of `skse64_loader.exe` and the real
//! launcher is kept as `SkyrimSELauncher.exe.corkscrew-orig`.
//!
//! Steam's "Verify integrity" and game updates restore the real launcher, so
//! [`apply`] runs before every script-extender launch and [`restore`] before
//! every plain launch and when the script extender is uninstalled.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use log::info;

/// Suffix of the kept original launcher.
pub const ORIGINAL_SUFFIX: &str = ".corkscrew-orig";

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

/// Whether the launcher is currently the script-extender loader.
pub fn is_applied(game_path: &Path, swap: LauncherSwap) -> bool {
    match (find(game_path, swap.launcher), find(game_path, swap.loader)) {
        (Some(launcher), Some(loader)) => same_contents(&launcher, &loader),
        _ => false,
    }
}

/// Put the loader in the launcher's place, keeping the real launcher.
/// Returns whether anything changed. Errors if the loader isn't installed.
pub fn apply(game_path: &Path, swap: LauncherSwap) -> io::Result<bool> {
    let loader = find(game_path, swap.loader).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} isn't installed in {}", swap.loader, game_path.display()),
        )
    })?;
    refuse_symlink(&loader)?;
    let launcher = find(game_path, swap.launcher).unwrap_or_else(|| game_path.join(swap.launcher));
    let original = PathBuf::from(format!("{}{ORIGINAL_SUFFIX}", launcher.display()));
    refuse_symlink(&launcher)?;
    refuse_symlink(&original)?;

    if launcher.exists() && same_contents(&launcher, &loader) {
        return Ok(false);
    }
    // The launcher on disk is a real one (first swap, or Steam restored it
    // after a verify/update): it becomes the kept original.
    if launcher.exists() {
        fs::rename(&launcher, &original)?;
    }
    fs::copy(&loader, &launcher)?;
    info!(
        "Script extender: {} now starts {} (original kept as {})",
        swap.launcher,
        swap.loader,
        original.display()
    );
    Ok(true)
}

/// Put the real launcher back. Returns whether anything changed.
pub fn restore(game_path: &Path, swap: LauncherSwap) -> io::Result<bool> {
    let launcher = find(game_path, swap.launcher).unwrap_or_else(|| game_path.join(swap.launcher));
    let original = PathBuf::from(format!("{}{ORIGINAL_SUFFIX}", launcher.display()));
    refuse_symlink(&launcher)?;
    refuse_symlink(&original)?;
    if !original.is_file() {
        return Ok(false);
    }
    let launcher_is_loader = match find(game_path, swap.loader) {
        Some(loader) => launcher.exists() && same_contents(&launcher, &loader),
        // The loader was removed (uninstall); a launcher that isn't the
        // original must be the old loader copy.
        None => launcher.exists() && !same_contents(&launcher, &original),
    };
    if launcher.exists() && !launcher_is_loader {
        // Steam already put a (possibly newer) real launcher back.
        fs::remove_file(&original)?;
        return Ok(true);
    }
    if launcher.exists() {
        fs::remove_file(&launcher)?;
    }
    fs::rename(&original, &launcher)?;
    info!("Script extender: restored the original {}", swap.launcher);
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
    fn skyrim_and_fallout4_have_swaps() {
        assert_eq!(for_game("skyrimse"), Some(SWAP));
        assert_eq!(for_game("fallout4").map(|s| s.loader), Some("f4se_loader.exe"));
        assert_eq!(for_game("hogwartslegacy"), None);
    }

    #[test]
    fn apply_then_restore_round_trips() {
        let tmp = game();
        let g = tmp.path();
        assert!(apply(g, SWAP).unwrap());
        assert_eq!(read(&g.join("SkyrimSELauncher.exe")), "skse loader");
        assert_eq!(read(&g.join("SkyrimSELauncher.exe.corkscrew-orig")), "launcher v1");
        assert!(is_applied(g, SWAP));
        assert!(!apply(g, SWAP).unwrap(), "second apply is a no-op");

        assert!(restore(g, SWAP).unwrap());
        assert_eq!(read(&g.join("SkyrimSELauncher.exe")), "launcher v1");
        assert!(!g.join("SkyrimSELauncher.exe.corkscrew-orig").exists());
        assert!(!restore(g, SWAP).unwrap(), "second restore is a no-op");
    }

    #[test]
    fn steam_verify_restoring_a_newer_launcher_is_kept() {
        let tmp = game();
        let g = tmp.path();
        apply(g, SWAP).unwrap();
        // Steam "Verify files" puts a (newer) real launcher back.
        fs::write(g.join("SkyrimSELauncher.exe"), "launcher v2").unwrap();
        assert!(!is_applied(g, SWAP));
        assert!(apply(g, SWAP).unwrap(), "re-applied before the next SKSE launch");
        assert_eq!(read(&g.join("SkyrimSELauncher.exe.corkscrew-orig")), "launcher v2");

        // Restoring after another verify keeps Steam's launcher.
        fs::write(g.join("SkyrimSELauncher.exe"), "launcher v3").unwrap();
        restore(g, SWAP).unwrap();
        assert_eq!(read(&g.join("SkyrimSELauncher.exe")), "launcher v3");
        assert!(!g.join("SkyrimSELauncher.exe.corkscrew-orig").exists());
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
