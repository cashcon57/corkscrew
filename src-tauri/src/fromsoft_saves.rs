//! Pre-launch FromSoft save backup.
//!
//! FromSoft titles store their saves as `*.sl2` files under
//! `%AppData%\Roaming\<GameName>\<numeric_user_id>\` inside the Wine bottle.
//! Wine has a long-running history of save corruption when the game is
//! interrupted (oom, kernel panic, ungraceful Crossover quit), so we copy
//! the current saves into `<saves_dir>/CorkscrewBackups/` before each
//! launch with a unix-timestamped filename, capped at `max_backups` total
//! per source file.
//!
//! This is a best-effort feature — failures must never block a launch.
//! Backup directory is `CorkscrewBackups/` rather than `Backups/` to make
//! it obvious we own these files vs. the user's own `Backups/` if any.
//!
//! Save filename conventions per game:
//! - Sekiro: `<id>/S0000.sl2` (the only save slot)
//! - Elden Ring: `<id>/ER0000.sl2`
//! - Dark Souls III: `<id>/DS30000.sl2`
//! - Dark Souls: Remastered: `<id>/DRAKS0005.sl2`
//! - Armored Core 6: `<id>/AC60000.sl2`
//!
//! We don't filter by exact filename — instead, every `*.sl2` (and the
//! associated `.bak` Souls writes alongside) under the user-id subdir is
//! considered.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::bottles::Bottle;

/// Default cap on retained backups per source file. Old backups beyond this
/// count are pruned oldest-first after each successful backup pass.
pub const DEFAULT_MAX_BACKUPS: usize = 10;

/// Subdirectory (under the per-user save dir) where Corkscrew stores backups.
/// Capitalized + namespaced so it's obvious in a file browser that these
/// are ours.
pub const BACKUP_SUBDIR: &str = "CorkscrewBackups";

/// One save file the user has on disk.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SaveFile {
    /// Absolute host path to the live save.
    pub path: PathBuf,
    /// Where the next backup of this save would land.
    pub backup_path: PathBuf,
    pub size_bytes: u64,
    pub modified_unix: u64,
}

// ---------------------------------------------------------------------------
// Path discovery
// ---------------------------------------------------------------------------

/// Map a game_id to the AppData\Roaming\<dirname> the game uses on Windows.
/// Returns None for non-FromSoft ids.
fn appdata_dirname_for(game_id: &str) -> Option<&'static str> {
    // DS:R saves live under Documents/NBGI/DARK SOULS REMASTERED, not
    // AppData/Roaming — needs a separate code path. Dropped here until
    // that's wired so we don't return a path the game never writes to.
    Some(match game_id {
        "sekiro" => "Sekiro",
        "eldenring" => "EldenRing",
        "darksouls3" => "DarkSoulsIII",
        "armoredcore6" => "ArmoredCore6",
        _ => return None,
    })
}

/// Locate the per-game saves directory inside the bottle.
///
/// We look under each bottle user directory's
/// `AppData/Roaming/<GameName>/`. The first existing match wins. The
/// returned path is the *parent* of the per-user numeric-id folders —
/// callers can then walk into each numeric subdir to find the actual saves.
pub fn find_fromsoft_saves_dir(bottle: &Bottle, game_id: &str) -> Option<PathBuf> {
    let appdata_name = appdata_dirname_for(game_id)?;
    let users = bottle.users_dir();
    if !users.is_dir() {
        return None;
    }

    let entries = fs::read_dir(&users).ok()?;
    for entry in entries.flatten() {
        let user = entry.path();
        if !user.is_dir() {
            continue;
        }
        let candidate = user
            .join("AppData")
            .join("Roaming")
            .join(appdata_name);
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

/// Enumerate `.sl2` save files under the saves directory.
pub fn list_saves(bottle: &Bottle, game_id: &str) -> Vec<SaveFile> {
    let Some(saves_dir) = find_fromsoft_saves_dir(bottle, game_id) else {
        return Vec::new();
    };
    list_saves_in_dir(&saves_dir)
}

/// Walk one level of subdirectories under `saves_dir` and collect any
/// `*.sl2` files. Skips our own backup subdir to avoid backing up backups.
fn list_saves_in_dir(saves_dir: &Path) -> Vec<SaveFile> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(saves_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        // Skip our own backup folder when listing live saves.
        if p.file_name().and_then(|n| n.to_str()) == Some(BACKUP_SUBDIR) {
            continue;
        }

        let Ok(inner) = fs::read_dir(&p) else { continue };
        for sub in inner.flatten() {
            let sp = sub.path();
            if !sp.is_file() {
                continue;
            }
            let Some(name) = sp.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.to_lowercase().ends_with(".sl2") {
                continue;
            }
            let meta = match sp.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let size = meta.len();
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let backup_dir = saves_dir.join(BACKUP_SUBDIR);
            let backup_name = format!("{}-{}.sl2.bak", name, modified);
            let backup_path = backup_dir.join(backup_name);
            out.push(SaveFile {
                path: sp,
                backup_path,
                size_bytes: size,
                modified_unix: modified,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Backup
// ---------------------------------------------------------------------------

/// Copy each live save into `<saves_dir>/CorkscrewBackups/` with a
/// timestamp-suffixed name, then prune backups per source file beyond
/// `max_backups`. Returns the number of files successfully backed up.
///
/// Errors during individual file copies are logged but do not abort the
/// pass. This is best-effort: a partial backup is better than none.
pub fn backup_saves_before_launch(
    bottle: &Bottle,
    game_id: &str,
    max_backups: usize,
) -> Result<usize, String> {
    let Some(saves_dir) = find_fromsoft_saves_dir(bottle, game_id) else {
        return Ok(0);
    };
    let backup_dir = saves_dir.join(BACKUP_SUBDIR);
    if let Err(e) = fs::create_dir_all(&backup_dir) {
        return Err(format!(
            "Failed to create backup dir {}: {}",
            backup_dir.display(),
            e
        ));
    }

    let saves = list_saves_in_dir(&saves_dir);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut copied = 0usize;
    for save in &saves {
        let Some(name) = save.path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let backup_name = format!("{}-{}.sl2.bak", name, now);
        let dest = backup_dir.join(&backup_name);
        match fs::copy(&save.path, &dest) {
            Ok(_) => copied += 1,
            Err(e) => {
                log::warn!(
                    "FromSoft save backup failed for {}: {}",
                    save.path.display(),
                    e
                );
                continue;
            }
        }
        prune_old_backups(&backup_dir, name, max_backups);
    }

    Ok(copied)
}

/// Keep at most `max` backups for `<source_name>` in `backup_dir`. Backup
/// filenames look like `<source_name>-<unix>.sl2.bak`.
fn prune_old_backups(backup_dir: &Path, source_name: &str, max: usize) {
    if max == 0 {
        return;
    }
    let prefix = format!("{}-", source_name);
    let suffix = ".sl2.bak";
    let entries = match fs::read_dir(backup_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut matches: Vec<(u64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let p = entry.path();
            let name = p.file_name()?.to_str()?;
            if !name.starts_with(&prefix) || !name.ends_with(suffix) {
                return None;
            }
            let stem = &name[prefix.len()..name.len() - suffix.len()];
            let ts: u64 = stem.parse().ok()?;
            Some((ts, p))
        })
        .collect();

    if matches.len() <= max {
        return;
    }
    // Sort newest-first, then drop the tail.
    matches.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in matches.into_iter().skip(max) {
        let _ = fs::remove_file(path);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bottle(path: PathBuf) -> Bottle {
        Bottle {
            name: "T".into(),
            path,
            source: "T".into(),
        }
    }

    fn write_save(saves_dir: &Path, user_id: &str, save_name: &str, body: &[u8]) -> PathBuf {
        let user_subdir = saves_dir.join(user_id);
        fs::create_dir_all(&user_subdir).unwrap();
        let p = user_subdir.join(save_name);
        fs::write(&p, body).unwrap();
        p
    }

    fn setup_bottle_with_saves(game_id: &str) -> (Bottle, PathBuf, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        let user = bottle_path
            .join("drive_c")
            .join("users")
            .join("crossover");
        let appdata_name = appdata_dirname_for(game_id).unwrap();
        let saves = user
            .join("AppData")
            .join("Roaming")
            .join(appdata_name);
        fs::create_dir_all(&saves).unwrap();
        (make_bottle(bottle_path), saves, tmp)
    }

    #[test]
    fn appdata_mapping_covers_supported_games() {
        for id in ["sekiro", "eldenring", "darksouls3", "armoredcore6"] {
            assert!(appdata_dirname_for(id).is_some(), "missing mapping for {}", id);
        }
        // DS:R saves under Documents/NBGI, not AppData/Roaming — until that
        // separate path is wired, we deliberately return None.
        assert!(appdata_dirname_for("darksouls_remastered").is_none());
        assert!(appdata_dirname_for("skyrimse").is_none());
    }

    #[test]
    fn find_dir_returns_none_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_bottle(tmp.path().join("EmptyBottle"));
        assert!(find_fromsoft_saves_dir(&bottle, "eldenring").is_none());
    }

    #[test]
    fn find_dir_per_game() {
        for (game, _appdata) in [
            ("sekiro", "Sekiro"),
            ("eldenring", "EldenRing"),
            ("darksouls3", "DarkSoulsIII"),
            ("armoredcore6", "ArmoredCore6"),
        ] {
            let (bottle, expected, _t) = setup_bottle_with_saves(game);
            let got = find_fromsoft_saves_dir(&bottle, game).expect("dir");
            assert_eq!(got, expected, "game {} dir mismatch", game);
        }
    }

    #[test]
    fn list_saves_finds_sl2_under_userid() {
        let (bottle, saves_dir, _t) = setup_bottle_with_saves("eldenring");
        write_save(&saves_dir, "76561198000000000", "ER0000.sl2", b"savedata");
        write_save(&saves_dir, "76561198000000000", "ignore.txt", b"x");

        let saves = list_saves(&bottle, "eldenring");
        assert_eq!(saves.len(), 1);
        assert!(saves[0].path.ends_with("ER0000.sl2"));
        assert_eq!(saves[0].size_bytes, "savedata".len() as u64);
    }

    #[test]
    fn backup_round_trip_and_retention() {
        let (bottle, saves_dir, _t) = setup_bottle_with_saves("sekiro");
        let _live = write_save(&saves_dir, "abc", "S0000.sl2", b"original");

        // First backup pass actually copies the live save into
        // CorkscrewBackups/. Returns 1 (one save copied).
        let n = backup_saves_before_launch(&bottle, "sekiro", 3).unwrap();
        assert_eq!(n, 1);
        let backup_dir = saves_dir.join(BACKUP_SUBDIR);
        assert!(backup_dir.is_dir());

        // Verify the real backup landed and its content matches the live save.
        let real_backups: Vec<_> = fs::read_dir(&backup_dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.starts_with("S0000.sl2-") && s.ends_with(".sl2.bak"))
                    .unwrap_or(false)
            })
            .collect();
        assert_eq!(real_backups.len(), 1);
        assert_eq!(fs::read(&real_backups[0]).unwrap(), b"original");

        // Pruning: synthesize five additional dated backups, then assert
        // retention only keeps the N highest timestamps. Use stable
        // synthetic values strictly larger than any real-time stamp so
        // they're always the "newest".
        let huge = u64::MAX - 10;
        for i in 0..5u64 {
            let synth = backup_dir.join(format!("S0000.sl2-{}.sl2.bak", huge + i));
            fs::write(&synth, b"x").unwrap();
        }

        // Now: 1 real + 5 synthetic = 6. Cap at 3 → keep three newest, all
        // synthetic.
        prune_old_backups(&backup_dir, "S0000.sl2", 3);
        let mut kept: Vec<u64> = fs::read_dir(&backup_dir)
            .unwrap()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                if !n.starts_with("S0000.sl2-") || !n.ends_with(".sl2.bak") {
                    return None;
                }
                let stem = &n["S0000.sl2-".len()..n.len() - ".sl2.bak".len()];
                stem.parse::<u64>().ok()
            })
            .collect();
        kept.sort_by(|a, b| b.cmp(a));
        assert_eq!(kept.len(), 3, "retention cap not honored");
        assert_eq!(kept, vec![huge + 4, huge + 3, huge + 2]);
    }

    #[test]
    fn backup_skips_our_own_backup_folder() {
        let (bottle, saves_dir, _t) = setup_bottle_with_saves("eldenring");
        write_save(&saves_dir, "u1", "ER0000.sl2", b"real");
        // Pre-create the backup dir + a junk .sl2 file that shouldn't be
        // considered a "live save".
        let backup_dir = saves_dir.join(BACKUP_SUBDIR);
        fs::create_dir_all(&backup_dir).unwrap();
        // The backup dir is a sibling of user-id dirs; list_saves walks
        // one level deep into each subdir of saves_dir — placing a .sl2
        // *inside* CorkscrewBackups would be reachable, so we explicitly
        // exclude that dir in list_saves_in_dir.
        fs::write(backup_dir.join("staleER0000.sl2"), b"junk").unwrap();

        let saves = list_saves(&bottle, "eldenring");
        assert_eq!(saves.len(), 1, "backup folder must be excluded");
        assert!(saves[0].path.ends_with("ER0000.sl2"));
    }

    #[test]
    fn backup_with_no_saves_returns_zero() {
        let (bottle, _saves_dir, _t) = setup_bottle_with_saves("armoredcore6");
        let n = backup_saves_before_launch(&bottle, "armoredcore6", 5).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn backup_for_nonfromsoft_game_returns_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_bottle(tmp.path().join("Bottle"));
        let n = backup_saves_before_launch(&bottle, "skyrimse", 5).unwrap();
        assert_eq!(n, 0);
    }
}
