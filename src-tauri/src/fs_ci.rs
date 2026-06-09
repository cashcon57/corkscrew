//! Case-insensitive filesystem lookups.
//!
//! Wine targets case-insensitive path semantics (NTFS, and APFS on macOS),
//! so any lookup of a game file authored with unknown casing must be
//! case-folded. These are the canonical implementations — the
//! `find_*_case_insensitive` helpers scattered across plugins and modules
//! delegate here so behavior can never diverge between callers.

use std::fs;
use std::path::{Path, PathBuf};

/// Find a regular file by name (case-insensitive) in `dir`, non-recursive.
/// Tries the exact-cased path first to avoid a directory scan.
pub fn find_file_ci(dir: &Path, target: &str) -> Option<PathBuf> {
    let exact = dir.join(target);
    if exact.is_file() {
        return Some(exact);
    }
    scan_ci(dir, target, |ft| ft.is_file())
}

/// Find a subdirectory by name (case-insensitive) in `dir`, non-recursive.
pub fn find_dir_ci(dir: &Path, target: &str) -> Option<PathBuf> {
    let exact = dir.join(target);
    if exact.is_dir() {
        return Some(exact);
    }
    scan_ci(dir, target, |ft| ft.is_dir())
}

/// Find any child entry (file or directory) by name (case-insensitive).
pub fn find_child_ci(parent: &Path, target: &str) -> Option<PathBuf> {
    let exact = parent.join(target);
    if exact.exists() {
        return Some(exact);
    }
    scan_ci(parent, target, |_| true)
}

fn scan_ci(
    dir: &Path,
    target: &str,
    type_ok: impl Fn(fs::FileType) -> bool,
) -> Option<PathBuf> {
    let target_lower = target.to_lowercase();
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if entry.file_type().map(&type_ok).unwrap_or(false)
            && entry.file_name().to_string_lossy().to_lowercase() == target_lower
        {
            return Some(entry.path());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_file_with_different_case() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("SkyrimSE.exe"), b"x").unwrap();
        // On a case-insensitive host (APFS) the exact-join fast path returns
        // the requested casing; on case-sensitive hosts the scan returns the
        // on-disk casing. Either way the file must be found.
        let found = find_file_ci(tmp.path(), "skyrimse.exe").unwrap();
        assert!(found
            .file_name()
            .unwrap()
            .to_string_lossy()
            .eq_ignore_ascii_case("SkyrimSE.exe"));
        assert!(find_file_ci(tmp.path(), "missing.exe").is_none());
    }

    #[test]
    fn file_lookup_ignores_directories() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("Data")).unwrap();
        assert!(find_file_ci(tmp.path(), "data").is_none());
        assert!(find_dir_ci(tmp.path(), "DATA").is_some());
    }

    #[test]
    fn child_lookup_matches_any_type() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("Common")).unwrap();
        std::fs::write(tmp.path().join("Game.exe"), b"x").unwrap();
        assert!(find_child_ci(tmp.path(), "common").is_some());
        assert!(find_child_ci(tmp.path(), "GAME.EXE").is_some());
        assert!(find_child_ci(tmp.path(), "missing").is_none());
    }
}
