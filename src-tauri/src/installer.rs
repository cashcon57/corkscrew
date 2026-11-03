use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use log::{debug, info, warn};
use rayon::prelude::*;
use thiserror::Error;
use walkdir::WalkDir;
use zip::ZipArchive;

/// Callback type for reporting extraction progress: (files_done, files_total).
pub type ExtractProgressCb = dyn Fn(u64, u64) + Send + Sync;

/// CP437 (DOS/OEM) high-byte to Unicode translation table.
/// Maps bytes 0x80..0xFF to their Unicode equivalents.
const CP437_HIGH: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å',
    'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ',
    'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»',
    '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐',
    '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧',
    '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀',
    'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩',
    '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', ' ',
];

/// Convert a CP437-encoded byte slice to a UTF-8 string.
/// Bytes 0x00..0x7F pass through as ASCII; 0x80..0xFF map via the table.
fn cp437_to_utf8(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len());
    for &b in bytes {
        if b < 0x80 {
            result.push(b as char);
        } else {
            result.push(CP437_HIGH[(b - 0x80) as usize]);
        }
    }
    result
}

/// Returns `true` if a path component or filename is macOS/archive junk that
/// should never be staged or deployed (`.DS_Store`, `__MACOSX`, `Thumbs.db`).
pub fn is_junk_file(path: &Path) -> bool {
    for component in path.components() {
        if let std::path::Component::Normal(s) = component {
            let s = s.to_string_lossy();
            if s == ".DS_Store" || s == "__MACOSX" || s == "Thumbs.db" || s == "desktop.ini" {
                return true;
            }
        }
    }
    false
}

/// Returns `true` if a relative path is FOMOD/NMM metadata or other mod
/// packaging artefacts that should never be deployed into the game directory.
/// This is a superset of [`is_junk_file`] — it additionally catches `fomod/`
/// directories, `meta.ini`, `info.xml`, and loose `ModuleConfig.xml` files.
pub fn is_deploy_junk(path: &Path) -> bool {
    if is_junk_file(path) {
        return true;
    }

    for component in path.components() {
        if let std::path::Component::Normal(s) = component {
            let s = s.to_string_lossy().to_lowercase();
            if s == "fomod" || s == "__fomod" {
                return true;
            }
        }
    }

    if let Some(name) = path.file_name() {
        let n = name.to_string_lossy().to_lowercase();
        if n == "meta.ini" || n == "info.xml" || n == "moduleconfig.xml" {
            return true;
        }
    }

    false
}

/// I/O buffer size for archive extraction (256 KiB).
/// Rust's default io::copy uses 8 KiB — 32x more syscalls per file.
const EXTRACT_BUF_SIZE: usize = 256 * 1024;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum InstallerError {
    #[error("Unsupported archive format: {0}")]
    UnsupportedFormat(String),

    #[error("Archive not found: {0}")]
    ArchiveNotFound(PathBuf),

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("ZIP extraction error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("7z extraction error: {0}")]
    SevenZ(String),

    #[error("RAR extraction error: {0}")]
    Rar(String),

    #[error("Tar extraction error: {0}")]
    Tar(String),

    #[error("WalkDir error: {0}")]
    WalkDir(#[from] walkdir::Error),

    /// Extraction was aborted because the caller's cancel check fired.
    /// The display string intentionally contains "Cancelled" — the
    /// collection pipeline uses it to classify the error as non-retryable.
    #[error("Extraction cancelled")]
    Cancelled,

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, InstallerError>;

/// Cancellation probe passed down into the extractors. Returning `true`
/// aborts the extraction at the next checkpoint (between entries and every
/// copy chunk; the native 7z child process is killed).
pub type CancelCheck<'a> = dyn Fn() -> bool + Send + Sync + 'a;

/// A `CancelCheck` that never cancels, for the non-cancellable entry points.
fn never_cancelled() -> bool {
    false
}

/// Normalize an archive entry name to `/` separators.
///
/// Archives built on Windows by some tools store `\` as the path separator
/// (the ZIP spec mandates `/`, but many archivers ignore it). On macOS/Linux
/// `\` is an ordinary filename character, so without this a single entry
/// `SKSE\Plugins\Foo.dll` would be written as ONE file with that literal
/// name instead of `SKSE/Plugins/Foo.dll`.
pub(crate) fn normalize_entry_name(name: &str) -> String {
    name.replace('\\', "/")
}

/// Convert a raw archive entry name into a safe path relative to the
/// extraction root, or `None` if the entry must be rejected.
///
/// The name is normalized with [`normalize_entry_name`] FIRST, so traversal
/// written with backslashes (`..\..\evil.txt`) is seen as `..` components —
/// on Unix the un-normalized string would be a single harmless-looking
/// component and slip past a component-based check. Rejects: `..`
/// components, absolute paths, Windows drive / UNC prefixes, NUL bytes, and
/// names that normalize to nothing. `.` and empty components are dropped.
pub(crate) fn safe_entry_path(raw_name: &str) -> Option<PathBuf> {
    let normalized = normalize_entry_name(raw_name);
    if normalized.contains('\0') || normalized.starts_with('/') {
        return None;
    }
    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return None; // drive letter, e.g. "C:/Windows/..."
    }
    let mut out = PathBuf::new();
    for component in normalized.split('/') {
        match component {
            "" | "." => continue,
            ".." => return None,
            c => out.push(c),
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Marker message for the I/O error produced by [`copy_with_cancel`].
const CANCELLED_IO_MSG: &str = "extraction cancelled";

/// `io::copy` that checks `cancel` between chunks so a multi-GB entry can be
/// interrupted promptly.
fn copy_with_cancel<R: io::Read + ?Sized, W: io::Write>(
    reader: &mut R,
    writer: &mut W,
    cancel: &CancelCheck,
) -> io::Result<u64> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        if cancel() {
            return Err(io::Error::other(CANCELLED_IO_MSG));
        }
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        writer.write_all(&buf[..n])?;
        total += n as u64;
    }
    writer.flush()?;
    Ok(total)
}

/// After a native (out-of-process) extraction, repair entries whose names
/// contain a literal `\` by moving them to the equivalent `/`-separated path.
///
/// Native tools (7z, unrar) may write `SKSE\Plugins\Foo.dll` as a single
/// file. Each such entry is validated with [`safe_entry_path`] before being
/// moved; unsafe names (e.g. `..\..\evil.txt`) are deleted. Symlinks with
/// backslash names are deleted rather than followed, and the destination
/// parent is canonicalized to confirm it stays inside `dest_dir`.
fn fix_backslash_entry_names(dest_dir: &Path) {
    let canonical_dest = match dest_dir.canonicalize() {
        Ok(p) => p,
        Err(_) => return,
    };
    // contents_first: children are handled before their (possibly
    // backslash-named) parent directory, which is then removed if empty.
    let entries: Vec<walkdir::DirEntry> = WalkDir::new(dest_dir)
        .min_depth(1)
        .contents_first(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .collect();

    for entry in entries {
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(dest_dir) else {
            continue;
        };
        let rel_str = rel.to_string_lossy();
        if !rel_str.contains('\\') {
            continue;
        }
        let file_type = entry.file_type();
        if file_type.is_dir() {
            // Directory with a backslash in its own name: its children were
            // already relocated (contents_first). Drop it if now empty.
            if entry.file_name().to_string_lossy().contains('\\') {
                let _ = fs::remove_dir(path);
            }
            continue;
        }
        if file_type.is_symlink() {
            warn!("Removing symlink with backslash name from archive: {}", rel_str);
            let _ = fs::remove_file(path);
            continue;
        }
        let Some(safe_rel) = safe_entry_path(&rel_str) else {
            warn!("Removing unsafe archive entry (path traversal): {}", rel_str);
            let _ = fs::remove_file(path);
            continue;
        };
        let target = dest_dir.join(&safe_rel);
        if let Some(parent) = target.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                warn!("Cannot create directory for '{}': {}", safe_rel.display(), e);
                continue;
            }
            match parent.canonicalize() {
                Ok(cp) if cp.starts_with(&canonical_dest) => {}
                _ => {
                    warn!(
                        "Removing archive entry whose target escapes the destination: {}",
                        rel_str
                    );
                    let _ = fs::remove_file(path);
                    continue;
                }
            }
        }
        if let Err(e) = fs::rename(path, &target) {
            warn!(
                "Failed to normalize backslash entry '{}' -> '{}': {}",
                rel_str,
                safe_rel.display(),
                e
            );
        } else {
            debug!("Normalized backslash entry: {} -> {}", rel_str, safe_rel.display());
        }
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// File extensions that are characteristic of mod content.
/// Used by `looks_like_mod_content()` to detect the data root inside archives.
const MOD_FILE_EXTENSIONS: &[&str] = &[
    // Bethesda (Skyrim, Fallout, Oblivion, Morrowind, Starfield)
    "esp", "esm", "esl", // plugin files
    "bsa", "ba2", // archives
    "nif", // meshes
    "dds", "tga",  // textures
    "hkx",  // animations
    "pex",  // compiled Papyrus scripts
    "seq",  // sequence files
    "swf",  // UI files
    "fuz",  // voice / lip-sync
    "dll",  // SKSE plugins
    "bin",  // SKSE address library data
    "ini",  // configuration files
    "json", // mod config / MCM settings
    // Unreal Engine (Hogwarts Legacy, Palworld, Ready or Not, etc.)
    "pak",  // UE PAK archives
    "ucas", // UE IO Store container
    "utoc", // UE IO Store table of contents
    "lua",  // UE4SS Lua scripts
];

/// Directory names that are characteristic of mod content.
const MOD_FOLDER_NAMES: &[&str] = &[
    // Bethesda
    "meshes",
    "textures",
    "scripts",
    "interface",
    "sound",
    "skse",
    // Unreal Engine
    "paks",
    "content",
    "mods",
    "logicmods",
    // Unity / BepInEx
    "bepinex",
    "bepinexpack",
    "plugins",
    "patchers",
    "monomod",
    // SMAPI / RimWorld
    "about",
    "assemblies",
];

// ---------------------------------------------------------------------------
// Archive extraction
// ---------------------------------------------------------------------------

/// Extract an archive into `dest_dir`, creating the directory if it does not
/// exist.  Returns the list of files that were extracted (absolute paths).
///
/// Supported formats (matched by file extension):
///   - `.zip`
///   - `.7z`
///
/// Detect archive format from magic bytes (first 8 bytes of file).
/// Returns `"zip"`, `"7z"`, `"rar"`, `"bsa"`, `"ba2"`, or `None` if unrecognized.
pub(crate) fn detect_format_from_magic(path: &Path) -> Option<&'static str> {
    let mut buf = [0u8; 8];
    let mut file = std::fs::File::open(path).ok()?;
    std::io::Read::read_exact(&mut file, &mut buf).ok()?;
    if buf[0] == 0x50 && buf[1] == 0x4B {
        Some("zip")
    } else if buf.len() >= 6 && buf[0..6] == [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C] {
        Some("7z")
    } else if buf[0..4] == [0x52, 0x61, 0x72, 0x21] {
        Some("rar")
    } else if buf[0..4] == [0x42, 0x53, 0x41, 0x00] {
        Some("bsa") // BSA\x00 — Bethesda BSA archive
    } else if buf[0..4] == [0x42, 0x54, 0x44, 0x58] {
        Some("ba2") // BTDX — Bethesda BA2 archive
    } else {
        None
    }
}

/// Try extracting with detected magic byte format (fallback path).
fn extract_by_magic(
    archive_path: &Path,
    dest_dir: &Path,
    cancel: &CancelCheck,
) -> Option<Result<Vec<PathBuf>>> {
    let detected = detect_format_from_magic(archive_path)?;
    log::info!(
        "Trying magic-byte-detected format '{}' for '{}'",
        detected,
        archive_path.display()
    );
    Some(match detected {
        "zip" => extract_zip_impl(archive_path, dest_dir, None, cancel),
        "7z" => extract_7z(archive_path, dest_dir, cancel),
        "rar" => extract_rar(archive_path, dest_dir, cancel),
        "bsa" | "ba2" => {
            log::info!(
                "Detected Bethesda archive ({}) — skipping general extraction",
                detected
            );
            return None; // BSA/BA2 handled by Wabbajack pipeline
        }
        _ => return None,
    })
}

pub fn extract_archive(archive_path: &Path, dest_dir: &Path) -> Result<Vec<PathBuf>> {
    extract_archive_cancellable(archive_path, dest_dir, &never_cancelled)
}

/// Like [`extract_archive`] but aborts with [`InstallerError::Cancelled`]
/// as soon as `cancel` returns `true` (checked between entries, between
/// copy chunks, and while a native 7z child runs — the child is killed).
/// A cancelled extraction may leave partial output in `dest_dir`; callers
/// own cleanup of that directory.
pub fn extract_archive_cancellable(
    archive_path: &Path,
    dest_dir: &Path,
    cancel: &CancelCheck,
) -> Result<Vec<PathBuf>> {
    if !archive_path.exists() {
        return Err(InstallerError::ArchiveNotFound(archive_path.to_path_buf()));
    }
    if cancel() {
        return Err(InstallerError::Cancelled);
    }

    fs::create_dir_all(dest_dir)?;

    // Check compound extensions first (tar.gz, tar.xz, tar.bz2)
    let name_lower = archive_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();

    if name_lower.ends_with(".tar.gz") || name_lower.ends_with(".tgz") {
        return extract_tar_gz(archive_path, dest_dir, cancel);
    }
    if name_lower.ends_with(".tar.xz") || name_lower.ends_with(".txz") {
        return extract_tar_xz(archive_path, dest_dir, cancel);
    }
    if name_lower.ends_with(".tar.bz2") || name_lower.ends_with(".tbz2") {
        return extract_tar_bz2(archive_path, dest_dir, cancel);
    }

    let ext = archive_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "zip" => extract_zip_impl(archive_path, dest_dir, None, cancel).or_else(|zip_err| {
            if matches!(zip_err, InstallerError::Cancelled) {
                return Err(zip_err);
            }
            // ZIP failed — check if file is actually a different format
            if let Some(actual) = detect_format_from_magic(archive_path) {
                if actual != "zip" {
                    log::warn!(
                        "File '{}' has .zip extension but magic bytes indicate {} — retrying",
                        archive_path.display(),
                        actual
                    );
                    // Clean failed extraction attempt before retrying
                    let _ = fs::remove_dir_all(dest_dir);
                    let _ = fs::create_dir_all(dest_dir);
                    return match actual {
                        "7z" => extract_7z(archive_path, dest_dir, cancel),
                        "rar" => extract_rar(archive_path, dest_dir, cancel),
                        _ => Err(zip_err),
                    };
                }
            }
            Err(zip_err)
        }),
        "7z" => extract_7z(archive_path, dest_dir, cancel),
        "rar" => extract_rar(archive_path, dest_dir, cancel),
        // Unknown extension — try magic bytes before giving up
        other => {
            if let Some(result) = extract_by_magic(archive_path, dest_dir, cancel) {
                result
            } else {
                Err(InstallerError::UnsupportedFormat(other.to_string()))
            }
        }
    }
}

/// Like [`extract_archive`] but reports per-file progress via a callback.
///
/// The callback receives `(files_done, files_total)`.  For ZIP archives
/// (the vast majority of Nexus mods), progress is reported after every few
/// files.  For other formats, progress is reported at start (0) and end.
pub fn extract_archive_with_progress(
    archive_path: &Path,
    dest_dir: &Path,
    progress: &ExtractProgressCb,
) -> Result<Vec<PathBuf>> {
    if !archive_path.exists() {
        return Err(InstallerError::ArchiveNotFound(archive_path.to_path_buf()));
    }

    fs::create_dir_all(dest_dir)?;

    let cancel: &CancelCheck = &never_cancelled;
    let name_lower = archive_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();

    // Non-ZIP formats have no per-file progress — just bookend 0 / done.
    let bookend = |f: &dyn Fn() -> Result<Vec<PathBuf>>| -> Result<Vec<PathBuf>> {
        progress(0, 1);
        let result = f();
        if let Ok(ref files) = result {
            progress(files.len() as u64, files.len() as u64);
        }
        result
    };

    if name_lower.ends_with(".tar.gz") || name_lower.ends_with(".tgz") {
        return bookend(&|| extract_tar_gz(archive_path, dest_dir, cancel));
    }
    if name_lower.ends_with(".tar.xz") || name_lower.ends_with(".txz") {
        return bookend(&|| extract_tar_xz(archive_path, dest_dir, cancel));
    }
    if name_lower.ends_with(".tar.bz2") || name_lower.ends_with(".tbz2") {
        return bookend(&|| extract_tar_bz2(archive_path, dest_dir, cancel));
    }

    let ext = archive_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "zip" => extract_zip_impl(archive_path, dest_dir, Some(progress), cancel).or_else(
            |zip_err| {
                if let Some(actual) = detect_format_from_magic(archive_path) {
                    if actual != "zip" {
                        log::warn!(
                            "File '{}' has .zip extension but magic bytes indicate {} — retrying",
                            archive_path.display(),
                            actual
                        );
                        let _ = fs::remove_dir_all(dest_dir);
                        let _ = fs::create_dir_all(dest_dir);
                        return match actual {
                            "7z" => bookend(&|| extract_7z(archive_path, dest_dir, cancel)),
                            "rar" => bookend(&|| extract_rar(archive_path, dest_dir, cancel)),
                            _ => Err(zip_err),
                        };
                    }
                }
                Err(zip_err)
            },
        ),
        "7z" => bookend(&|| extract_7z(archive_path, dest_dir, cancel)),
        "rar" => bookend(&|| extract_rar(archive_path, dest_dir, cancel)),
        // Unknown extension — try magic bytes before giving up
        other => match detect_format_from_magic(archive_path) {
            Some(detected) => {
                log::info!(
                    "Unknown extension '{}', magic bytes indicate {} — extracting",
                    other,
                    detected
                );
                match detected {
                    "zip" => extract_zip_impl(archive_path, dest_dir, Some(progress), cancel),
                    "7z" => bookend(&|| extract_7z(archive_path, dest_dir, cancel)),
                    "rar" => bookend(&|| extract_rar(archive_path, dest_dir, cancel)),
                    _ => Err(InstallerError::UnsupportedFormat(other.to_string())),
                }
            }
            None => Err(InstallerError::UnsupportedFormat(other.to_string())),
        },
    }
}

/// Extract a `.zip` archive (no progress, not cancellable).
#[cfg(test)]
fn extract_zip(archive_path: &Path, dest_dir: &Path) -> Result<Vec<PathBuf>> {
    extract_zip_impl(archive_path, dest_dir, None, &never_cancelled)
}

/// Extract a `.zip` archive using the `zip` crate with parallel file extraction.
///
/// Uses a two-pass approach:
/// 1. Sequential scan: read central directory, validate + normalize entry
///    names, create directories
/// 2. Parallel extraction via rayon: each thread opens its own ZipArchive handle
///    and extracts assigned entries independently (ZIP supports random access)
///
/// Rayon's work-stealing pool automatically balances: when many archives extract
/// concurrently each gets ~1 thread; when only the last few remain, they get all cores.
///
/// When `progress` is provided it receives throttled `(files_done, total)`
/// updates. `cancel` is polled per entry and per copy chunk.
fn extract_zip_impl(
    archive_path: &Path,
    dest_dir: &Path,
    progress: Option<&ExtractProgressCb>,
    cancel: &CancelCheck,
) -> Result<Vec<PathBuf>> {
    let file = io::BufReader::with_capacity(EXTRACT_BUF_SIZE, fs::File::open(archive_path)?);
    let mut archive = ZipArchive::new(file)?;

    // Pass 1: collect file entries and create all directories up front
    let mut file_entries: Vec<(usize, PathBuf)> = Vec::new();

    for i in 0..archive.len() {
        if cancel() {
            return Err(InstallerError::Cancelled);
        }
        let entry = archive.by_index(i)?;

        // Reject raw filenames with null bytes before any conversion
        let raw_name = entry.name_raw();
        if raw_name.contains(&0u8) {
            warn!("Skipping ZIP entry with null byte in raw filename");
            continue;
        }

        // Reject symlink entries — a malicious archive can plant a symlink
        // and then write through it to an arbitrary host path. The
        // component-based traversal check below does NOT catch this. The
        // tar path already does the equivalent check (see below in this
        // file). Detect symlinks via the POSIX unix mode S_IFLNK bits in
        // the central-directory external attributes.
        if !entry.is_dir()
            && entry
                .unix_mode()
                .map_or(false, |m| m & 0o170000 == 0o120000)
        {
            warn!(
                "Skipping symlink entry in ZIP archive: {}",
                String::from_utf8_lossy(raw_name)
            );
            continue;
        }

        // Decode filename, preferring UTF-8 over CP437.
        //
        // ZIP spec: when general-purpose bit 11 is set the filename is UTF-8;
        // when unset it's CP437. Modern archives produced by anything except
        // legacy DOS tools use UTF-8 even without setting the bit. Probing
        // UTF-8 first protects valid `café.txt`-style filenames from being
        // corrupted by an unconditional CP437 decode of any byte >= 0x80.
        let decoded_name = if raw_name.iter().all(|&b| b < 0x80) {
            // ASCII is valid UTF-8, so this never substitutes characters.
            String::from_utf8_lossy(raw_name).into_owned()
        } else {
            match std::str::from_utf8(raw_name) {
                Ok(s) => s.to_string(),
                Err(_) => {
                    let converted = cp437_to_utf8(raw_name);
                    debug!("CP437 filename converted: {:?} -> {}", raw_name, converted);
                    converted
                }
            }
        };

        // Normalize `\` separators and reject traversal BEFORE building any
        // output path (see `safe_entry_path`).
        let Some(relative) = safe_entry_path(&decoded_name) else {
            if !decoded_name
                .trim_matches(|c| c == '/' || c == '\\' || c == '.')
                .is_empty()
            {
                warn!("Skipping unsafe ZIP entry (path traversal): {}", decoded_name);
            }
            continue;
        };
        let out_path = dest_dir.join(&relative);

        if is_junk_file(&relative) {
            continue;
        }

        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            file_entries.push((i, out_path));
        }
    }

    // Drop the first archive handle before parallel extraction
    drop(archive);

    let total_files = file_entries.len() as u64;
    if let Some(cb) = progress {
        cb(0, total_files);
    }
    // Shared atomic counter for cross-thread progress tracking
    let files_done = AtomicU64::new(0);

    // Pass 2: parallel extraction — partition entries into chunks so each rayon
    // thread opens ONE ZipArchive handle and extracts all its entries, avoiding
    // the overhead of re-parsing the central directory per file.
    let num_threads = rayon::current_num_threads().max(1);
    let archive_path_owned = archive_path.to_path_buf();
    let chunks: Vec<&[(usize, PathBuf)]> = file_entries
        .chunks((file_entries.len() / num_threads).max(1))
        .collect();

    let results: Vec<Vec<std::result::Result<PathBuf, String>>> = chunks
        .par_iter()
        .map(|chunk| {
            // Open one archive handle per thread chunk
            let f = match fs::File::open(&archive_path_owned) {
                Ok(file) => io::BufReader::with_capacity(EXTRACT_BUF_SIZE, file),
                Err(e) => {
                    return chunk
                        .iter()
                        .map(|_| Err(format!("Open failed: {}", e)))
                        .collect();
                }
            };
            let mut arch = match ZipArchive::new(f) {
                Ok(a) => a,
                Err(e) => {
                    return chunk
                        .iter()
                        .map(|_| Err(format!("ZIP parse failed: {}", e)))
                        .collect();
                }
            };
            chunk
                .iter()
                .map(|(idx, out_path)| {
                    if cancel() {
                        return Err(CANCELLED_IO_MSG.to_string());
                    }
                    let mut entry = arch.by_index(*idx).map_err(|e| e.to_string())?;
                    let mut out_file = io::BufWriter::with_capacity(
                        EXTRACT_BUF_SIZE,
                        fs::File::create(out_path).map_err(|e| e.to_string())?,
                    );
                    copy_with_cancel(&mut entry, &mut out_file, cancel)
                        .map_err(|e| e.to_string())?;

                    if let Some(cb) = progress {
                        let done = files_done.fetch_add(1, Ordering::Relaxed) + 1;
                        // Throttle progress callbacks — emit every ~2% or every 10 files
                        let interval = (total_files / 50).clamp(10, 100);
                        if done.is_multiple_of(interval) || done == total_files {
                            cb(done, total_files);
                        }
                    }

                    Ok(out_path.clone())
                })
                .collect()
        })
        .collect();

    if cancel() {
        return Err(InstallerError::Cancelled);
    }

    let mut extracted = Vec::with_capacity(file_entries.len());
    for chunk_results in results {
        for r in chunk_results {
            match r {
                Ok(p) => extracted.push(p),
                Err(e) => warn!("Failed to extract ZIP entry: {}", e),
            }
        }
    }

    // Final progress callback to ensure we report 100%
    if let Some(cb) = progress {
        cb(extracted.len() as u64, total_files);
    }

    info!(
        "Extracted {} files from ZIP: {}",
        extracted.len(),
        archive_path.display()
    );
    Ok(extracted)
}

/// Find a native `7z` / `7zz` / `7za` binary on the system.
///
/// Checks well-known Homebrew and distro paths before falling back to a
/// bare `$PATH` lookup.  Returns `None` if no binary is found.
fn find_native_7z() -> Option<PathBuf> {
    // Candidates in priority order: 7zz (latest official), 7z (p7zip), 7za (standalone)
    let names = ["7zz", "7z", "7za"];

    // Well-known locations (Homebrew Apple Silicon, Homebrew Intel, Linux distro)
    let prefixes: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"];

    for name in &names {
        for prefix in prefixes {
            let candidate = PathBuf::from(prefix).join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // Fall back to $PATH lookup
    for name in &names {
        if let Ok(output) = Command::new("which").arg(name).output() {
            if output.status.success() {
                let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path_str.is_empty() {
                    return Some(PathBuf::from(path_str));
                }
            }
        }
    }

    None
}

/// Extract a `.7z` archive using the native `7z` binary if available,
/// falling back to the `sevenz-rust2` crate (pure-Rust, much slower).
///
/// Native `7z` / `7zz` (C implementation) is 10–100x faster than the pure-Rust
/// LZMA decoder for solid archives — the difference between seconds and tens of
/// minutes for large texture mods.
fn extract_7z(archive_path: &Path, dest_dir: &Path, cancel: &CancelCheck) -> Result<Vec<PathBuf>> {
    if let Some(bin) = find_native_7z() {
        match extract_7z_native(&bin, archive_path, dest_dir, cancel) {
            Ok(files) => return Ok(files),
            // Never fall back to the (slow) pure-Rust decoder after a cancel.
            Err(InstallerError::Cancelled) => return Err(InstallerError::Cancelled),
            Err(e) => {
                warn!(
                    "Native 7z extraction failed ({}), falling back to pure-Rust: {}",
                    bin.display(),
                    e
                );
            }
        }
    } else {
        info!(
            "No native 7z binary found — using pure-Rust decoder (slower). \
             Install p7zip for faster extraction: brew install p7zip (macOS) \
             or sudo apt install p7zip-full (Linux)."
        );
    }

    extract_7z_rust(archive_path, dest_dir, cancel)
}

/// Extract using the native `7z` / `7zz` command-line tool.
///
/// The child is polled rather than waited on so `cancel` can kill it; stderr
/// is drained on a helper thread so a chatty child can never block on a full
/// pipe. Entries the tool wrote with literal `\` in their names are then
/// relocated / validated by [`fix_backslash_entry_names`].
fn extract_7z_native(
    bin: &Path,
    archive_path: &Path,
    dest_dir: &Path,
    cancel: &CancelCheck,
) -> Result<Vec<PathBuf>> {
    use std::io::Read as _;
    use std::process::Stdio;

    info!(
        "Extracting 7z with native binary {}: {} -> {}",
        bin.display(),
        archive_path.display(),
        dest_dir.display()
    );

    let mut child = Command::new(bin)
        .arg("x") // eXtract with full paths
        .arg(archive_path)
        .arg(format!("-o{}", dest_dir.display())) // output directory
        .arg("-y") // assume Yes to all prompts
        .arg("-bso0") // suppress normal stdout
        .arg("-bsp0") // suppress progress stdout
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| InstallerError::SevenZ(format!("Failed to run {}: {}", bin.display(), e)))?;

    let stderr_reader = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = pipe.read_to_string(&mut buf);
            buf
        })
    });

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if cancel() {
                    info!("Cancelling native 7z extraction of {}", archive_path.display());
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(InstallerError::Cancelled);
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(InstallerError::SevenZ(format!(
                    "Failed waiting for {}: {}",
                    bin.display(),
                    e
                )));
            }
        }
    };
    let stderr = stderr_reader
        .and_then(|h| h.join().ok())
        .unwrap_or_default();

    if !status.success() {
        return Err(InstallerError::SevenZ(format!(
            "7z exited with {}: {}",
            status,
            stderr.trim()
        )));
    }

    fix_backslash_entry_names(dest_dir);

    // Walk destination to collect extracted files (same as Rust path)
    collect_extracted_files(dest_dir)
}

/// Extract using the pure-Rust `sevenz-rust2` crate (slow fallback).
///
/// Uses a custom per-entry extract function so every entry name is
/// normalized and validated BEFORE anything is written (the crate's default
/// `dest.join(entry.name())` would write `..\..\x` / `SKSE\Plugins\x.dll`
/// verbatim), and so `cancel` is honoured between entries and copy chunks.
fn extract_7z_rust(
    archive_path: &Path,
    dest_dir: &Path,
    cancel: &CancelCheck,
) -> Result<Vec<PathBuf>> {
    info!(
        "Extracting 7z with pure-Rust decoder: {}",
        archive_path.display()
    );

    let mut extracted: Vec<PathBuf> = Vec::new();
    let mut cancelled = false;

    let result = sevenz_rust2::decompress_file_with_extract_fn(
        archive_path,
        dest_dir,
        |entry, reader, _unchecked_dest| {
            if cancel() {
                cancelled = true;
                return Err(sevenz_rust2::Error::Other(CANCELLED_IO_MSG.into()));
            }

            // Entries we skip must still be drained: in a solid block the
            // next entry's reader starts where this one stops.
            let skip = |reader: &mut dyn io::Read| -> std::result::Result<bool, sevenz_rust2::Error> {
                io::copy(reader, &mut io::sink())?;
                Ok(true)
            };

            let Some(relative) = safe_entry_path(entry.name()) else {
                if !entry.name().trim_matches(|c| c == '/' || c == '\\' || c == '.').is_empty() {
                    warn!("Skipping unsafe 7z entry (path traversal): {}", entry.name());
                }
                return skip(reader);
            };
            if entry.is_anti_item() || is_junk_file(&relative) {
                return skip(reader);
            }

            let out_path = dest_dir.join(&relative);
            if entry.is_directory() {
                fs::create_dir_all(&out_path)?;
                return Ok(true);
            }
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out_file =
                io::BufWriter::with_capacity(EXTRACT_BUF_SIZE, fs::File::create(&out_path)?);
            if let Err(e) = copy_with_cancel(reader, &mut out_file, cancel) {
                if cancel() {
                    cancelled = true;
                }
                return Err(e.into());
            }
            extracted.push(out_path);
            Ok(true)
        },
    );

    if cancelled || cancel() {
        return Err(InstallerError::Cancelled);
    }
    result.map_err(|e| {
        InstallerError::SevenZ(format!(
            "Failed to extract 7z {}: {}",
            archive_path.display(),
            e
        ))
    })?;

    info!(
        "Extracted {} files from 7z: {}",
        extracted.len(),
        archive_path.display()
    );
    Ok(extracted)
}

/// Walk `dest_dir` and collect all extracted files, rejecting any that
/// escape the destination directory (path traversal / symlink attacks).
fn collect_extracted_files(dest_dir: &Path) -> Result<Vec<PathBuf>> {
    let canonical_dest = dest_dir
        .canonicalize()
        .unwrap_or_else(|_| dest_dir.to_path_buf());

    let mut extracted: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(dest_dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let path = entry.into_path();
            // Remove junk files that were extracted by native 7z
            if is_junk_file(&path) {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            if let Ok(canonical) = path.canonicalize() {
                if canonical.starts_with(&canonical_dest) {
                    extracted.push(path);
                } else {
                    warn!(
                        "Skipping 7z entry outside destination: {}",
                        canonical.display()
                    );
                    let _ = std::fs::remove_file(&path);
                }
            } else {
                warn!(
                    "Skipping 7z entry that failed canonicalization: {}",
                    path.display()
                );
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    info!(
        "Extracted {} files from 7z: {}",
        extracted.len(),
        dest_dir.display()
    );
    Ok(extracted)
}

/// Extract a `.rar` archive using the `unrar` crate.
fn extract_rar(archive_path: &Path, dest_dir: &Path, cancel: &CancelCheck) -> Result<Vec<PathBuf>> {
    let mut extracted = Vec::new();
    let archive = unrar::Archive::new(archive_path)
        .open_for_processing()
        .map_err(|e| InstallerError::Rar(e.to_string()))?;

    let mut cursor = Some(
        archive
            .read_header()
            .map_err(|e| InstallerError::Rar(e.to_string()))?,
    );

    let canonical_dest = dest_dir
        .canonicalize()
        .unwrap_or_else(|_| dest_dir.to_path_buf());

    while let Some(Some(header)) = cursor.take() {
        if cancel() {
            return Err(InstallerError::Cancelled);
        }
        let entry = header.entry();

        // Normalize `\` separators and reject traversal before building the
        // output path (see `safe_entry_path`).
        let raw_name = entry.filename.to_string_lossy().into_owned();
        let relative = match safe_entry_path(&raw_name) {
            Some(rel) if !is_junk_file(&rel) => rel,
            other => {
                if other.is_none() {
                    warn!("Skipping RAR entry with path traversal: {}", raw_name);
                }
                let next = header
                    .skip()
                    .map_err(|e| InstallerError::Rar(e.to_string()))?;
                cursor = Some(
                    next.read_header()
                        .map_err(|e| InstallerError::Rar(e.to_string()))?,
                );
                continue;
            }
        };

        let out_path = dest_dir.join(&relative);

        if entry.is_file() {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let result = header
                .extract_to(&out_path)
                .map_err(|e| InstallerError::Rar(e.to_string()))?;
            // Post-extraction canonicalization check (catches symlink escapes)
            if let Ok(canonical) = out_path.canonicalize() {
                if canonical.starts_with(&canonical_dest) {
                    extracted.push(out_path);
                } else {
                    warn!(
                        "Removing RAR entry outside destination: {}",
                        canonical.display()
                    );
                    let _ = fs::remove_file(&out_path);
                }
            } else {
                warn!(
                    "Skipping RAR entry that failed canonicalization: {}",
                    out_path.display()
                );
                let _ = fs::remove_file(&out_path);
            }
            cursor = Some(
                result
                    .read_header()
                    .map_err(|e| InstallerError::Rar(e.to_string()))?,
            );
        } else {
            fs::create_dir_all(&out_path)?;
            let next = header
                .skip()
                .map_err(|e| InstallerError::Rar(e.to_string()))?;
            cursor = Some(
                next.read_header()
                    .map_err(|e| InstallerError::Rar(e.to_string()))?,
            );
        }
    }

    info!(
        "Extracted {} files from RAR: {}",
        extracted.len(),
        archive_path.display()
    );
    Ok(extracted)
}

/// Extract a `.tar.gz` / `.tgz` archive.
fn extract_tar_gz(archive_path: &Path, dest_dir: &Path, cancel: &CancelCheck) -> Result<Vec<PathBuf>> {
    let file = io::BufReader::with_capacity(EXTRACT_BUF_SIZE, fs::File::open(archive_path)?);
    let decoder = flate2::read::GzDecoder::new(file);
    extract_tar(decoder, archive_path, dest_dir, cancel)
}

/// Extract a `.tar.xz` / `.txz` archive.
fn extract_tar_xz(archive_path: &Path, dest_dir: &Path, cancel: &CancelCheck) -> Result<Vec<PathBuf>> {
    let file = io::BufReader::with_capacity(EXTRACT_BUF_SIZE, fs::File::open(archive_path)?);
    let decoder = xz2::read::XzDecoder::new(file);
    extract_tar(decoder, archive_path, dest_dir, cancel)
}

/// Extract a `.tar.bz2` / `.tbz2` archive.
fn extract_tar_bz2(archive_path: &Path, dest_dir: &Path, cancel: &CancelCheck) -> Result<Vec<PathBuf>> {
    let file = io::BufReader::with_capacity(EXTRACT_BUF_SIZE, fs::File::open(archive_path)?);
    let decoder = bzip2::read::BzDecoder::new(file);
    extract_tar(decoder, archive_path, dest_dir, cancel)
}

/// Shared tar extraction logic for any decompressed reader.
/// TAR is a stream format so parallelism isn't possible, but buffered I/O helps.
fn extract_tar<R: io::Read>(
    reader: R,
    archive_path: &Path,
    dest_dir: &Path,
    cancel: &CancelCheck,
) -> Result<Vec<PathBuf>> {
    let buffered = io::BufReader::with_capacity(EXTRACT_BUF_SIZE, reader);
    let mut archive = tar::Archive::new(buffered);
    let mut extracted = Vec::new();

    for entry_result in archive
        .entries()
        .map_err(|e| InstallerError::Tar(e.to_string()))?
    {
        if cancel() {
            return Err(InstallerError::Cancelled);
        }
        let mut entry = entry_result.map_err(|e| InstallerError::Tar(e.to_string()))?;
        let rel_path = entry
            .path()
            .map_err(|e| InstallerError::Tar(e.to_string()))?
            .into_owned();

        // Component-level traversal check. The previous `starts_with(dest_dir)`
        // check was lexical only — `Data/../../outside` would pass it because
        // the string prefix matches before path normalization.
        if rel_path
            .components()
            .any(|c| matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            ))
        {
            warn!(
                "Skipping tar path traversal attempt: {}",
                rel_path.display()
            );
            continue;
        }

        let out_path = dest_dir.join(&rel_path);

        // Defense in depth: also keep the prefix check for any edge cases the
        // component scan misses.
        if !out_path.starts_with(dest_dir) {
            warn!(
                "Skipping tar entry outside dest_dir: {}",
                rel_path.display()
            );
            continue;
        }

        // Reject symlinks to prevent symlink-based escape attacks
        if entry.header().entry_type().is_symlink() {
            warn!("Skipping symlink in tar archive: {}", rel_path.display());
            continue;
        }

        if is_junk_file(&rel_path) {
            continue;
        }

        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&out_path)?;
        } else if entry.header().entry_type().is_file() {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out_file =
                io::BufWriter::with_capacity(EXTRACT_BUF_SIZE, fs::File::create(&out_path)?);
            if let Err(e) = copy_with_cancel(&mut entry, &mut out_file, cancel) {
                if cancel() {
                    return Err(InstallerError::Cancelled);
                }
                return Err(e.into());
            }
            extracted.push(out_path);
        }
    }

    info!(
        "Extracted {} files from tar archive: {}",
        extracted.len(),
        archive_path.display()
    );
    Ok(extracted)
}

// ---------------------------------------------------------------------------
// Mod content detection
// ---------------------------------------------------------------------------

/// Heuristic: does `directory` look like it already contains mod content
/// for any supported game family?
///
/// Recognized signals:
/// - Bethesda subdirs (meshes, textures, scripts, skse, etc.)
/// - Unreal Engine subdirs (paks, content, logicmods)
/// - Unity / BepInEx subdirs (bepinex, bepinexpack, plugins, patchers)
/// - SMAPI / RimWorld subdirs (about, assemblies, mods)
/// - Mod file extensions (esp/esm/bsa/pak/dll/lua)
/// - Loose `manifest.json` (SMAPI / Thunderstore convention)
/// - Loose `.pak` files (UE)
/// - Loose `About.xml` is covered via the `about` directory check
fn looks_like_mod_content(directory: &Path) -> bool {
    let entries = match fs::read_dir(directory) {
        Ok(rd) => rd,
        Err(_) => return false,
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_lowercase();

        // Check for characteristic sub-directories.
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
            && MOD_FOLDER_NAMES.contains(&name_str.as_str())
        {
            return true;
        }

        // Check for characteristic file extensions.
        if let Some(ext) = Path::new(&name).extension().and_then(|e| e.to_str()) {
            if MOD_FILE_EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
                return true;
            }
        }

        // SMAPI / Thunderstore signal: a loose `manifest.json` is a strong
        // indicator that this directory IS the mod root and should not be
        // recursed into. (Inside SMAPI mods the manifest sits next to the
        // assembly DLL, so the .dll extension would also catch it — this is
        // belt-and-suspenders for Thunderstore-style packages where the
        // payload is just config + manifest.)
        if name_str == "manifest.json" {
            return true;
        }
    }

    false
}

/// Locate the actual root of mod content inside `extracted_dir`.
///
/// Many archives nest content in a single wrapper directory or include an
/// explicit `Data` folder.  This function walks down into that structure so
/// the caller can copy only the relevant files.
///
/// Recognized shapes (Bethesda, Unreal Engine, Unity/BepInEx, SMAPI,
/// RimWorld) — see [`looks_like_mod_content`] for the full signal list.
///
/// Rules (evaluated in order, at each wrapper level):
/// 0. If the directory contains `fomod/ModuleConfig.xml` -> return it
///    (FOMOD source paths are relative to this directory).
/// 1. If the extracted root has a single top-level directory:
///    a. If that directory is named "data" (case-insensitive) -> use its
///    *contents* (i.e. return the Data dir itself so the caller copies
///    children).
///    b. If that directory looks like mod content -> use it.
///    c. Recurse: treat that directory as the new root and re-evaluate.
/// 2. If the extracted root contains a child named "Data" -> return that.
/// 3. If the extracted root looks like mod content -> return it.
/// 4. Default -> return the extracted root unchanged.
pub fn find_data_root(extracted_dir: &Path) -> PathBuf {
    _find_data_root_inner(extracted_dir, 0)
}

/// Check if an extracted archive contains a "Root" folder (MO2 convention).
/// Returns `(root_folder, data_root)` — root_folder is `Some(path)` if a
/// case-insensitive "Root" child directory exists. Files inside it should be
/// deployed to the game root folder, not the Data folder.
pub fn split_root_and_data(extracted_dir: &Path) -> (Option<PathBuf>, PathBuf) {
    let entries: Vec<_> = std::fs::read_dir(extracted_dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .collect();

    let root_folder = entries.iter().find(|e| {
        e.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
            && e.file_name().to_string_lossy().to_lowercase() == "root"
    });

    if let Some(rf) = root_folder {
        // If there's a "Root" folder, the data root is the rest of the archive
        let data_root = find_data_root(extracted_dir);
        (Some(rf.path()), data_root)
    } else {
        (None, find_data_root(extracted_dir))
    }
}

/// Does this directory listing contain `fomod/ModuleConfig.xml`
/// (both names case-insensitive)?
fn has_fomod_config(entries: &[fs::DirEntry]) -> bool {
    entries.iter().any(|e| {
        e.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
            && e.file_name().to_string_lossy().eq_ignore_ascii_case("fomod")
            && fs::read_dir(e.path())
                .map(|rd| {
                    rd.filter_map(|c| c.ok()).any(|c| {
                        c.file_type().map(|ft| ft.is_file()).unwrap_or(false)
                            && c.file_name()
                                .to_string_lossy()
                                .eq_ignore_ascii_case("ModuleConfig.xml")
                    })
                })
                .unwrap_or(false)
    })
}

/// Inner recursive helper with a depth guard to avoid infinite loops on
/// pathological archives.
fn _find_data_root_inner(dir: &Path, depth: u32) -> PathBuf {
    const MAX_DEPTH: u32 = 10;
    if depth > MAX_DEPTH {
        return dir.to_path_buf();
    }

    // Collect top-level entries (skip hidden files).
    let top_level: Vec<_> = fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .collect();

    // --- Rule 0: FOMOD installer root ---
    // FOMOD source paths in ModuleConfig.xml are relative to the directory
    // holding `fomod/`, so that directory IS the root — even when it also
    // has a `Data/` child (Rule 2 would otherwise pick `Data/`, dropping
    // `fomod/` and every option folder, and the FOMOD would never be seen).
    if has_fomod_config(&top_level) {
        debug!("find_data_root: FOMOD installer root -> {}", dir.display());
        return dir.to_path_buf();
    }

    // --- Rule 1: single top-level directory ---
    if top_level.len() == 1 {
        let entry = &top_level[0];
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            let entry_path = entry.path();

            // 1a – named "data"
            if name == "data" {
                debug!(
                    "find_data_root: single dir named 'data' -> {}",
                    entry_path.display()
                );
                return entry_path;
            }

            // 1b – if the single directory IS a recognized mod folder name
            // (e.g. "skse", "meshes", "textures", "bepinex"), the CURRENT
            // directory is the data root — do NOT recurse into it or we'll
            // strip the folder prefix. This MUST run before the
            // looks_like_mod_content check below: a wrapper directory like
            // `BepInEx/` contains `plugins/` (also a mod-folder name), so
            // looks_like_mod_content would otherwise return the wrapper
            // and the deploy would lose the `BepInEx/` prefix.
            if MOD_FOLDER_NAMES.contains(&name.as_str()) {
                debug!(
                    "find_data_root: single dir '{}' is a known mod folder -> {}",
                    name,
                    dir.display()
                );
                return dir.to_path_buf();
            }

            // 1c – looks like mod content
            if looks_like_mod_content(&entry_path) {
                debug!(
                    "find_data_root: single dir looks like mod content -> {}",
                    entry_path.display()
                );
                return entry_path;
            }

            // Otherwise recurse into the wrapper directory
            return _find_data_root_inner(&entry_path, depth + 1);
        }
    }

    // --- Rule 2: child named "Data" (case-insensitive) ---
    for entry in &top_level {
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name == "data" {
                debug!(
                    "find_data_root: found Data subfolder -> {}",
                    entry.path().display()
                );
                return entry.path();
            }
        }
    }

    // --- Rule 3: extracted root itself looks like mod content ---
    if looks_like_mod_content(dir) {
        debug!(
            "find_data_root: extracted root looks like mod content -> {}",
            dir.display()
        );
        return dir.to_path_buf();
    }

    // --- Rule 4: fallback ---
    debug!(
        "find_data_root: falling back to extracted root -> {}",
        dir.display()
    );
    dir.to_path_buf()
}

// ---------------------------------------------------------------------------
// Install / uninstall
// ---------------------------------------------------------------------------

/// Install a mod from an archive into the game's `Data` directory.
///
/// 1. Extracts the archive into a temporary directory.
/// 2. Determines the actual data root within the extracted tree.
/// 3. Copies every file from the data root into `data_dir`, preserving the
///    relative directory structure.
///
/// Returns the list of installed files as **relative** paths (relative to
/// `data_dir`), suitable for storing in a database so they can be removed
/// later by [`uninstall_mod_files`].
///
/// The caller is responsible for persisting metadata (mod name, version,
/// Nexus ID, installed file list) to the database.
pub fn install_mod(
    archive_path: &Path,
    data_dir: &Path,
    _mod_name: &str,
    _mod_version: &str,
    _nexus_mod_id: Option<i64>,
) -> Result<Vec<String>> {
    // 1. Extract into a unique temp directory. Use `tempfile::Builder` so
    // concurrent installs in the same process don't collide on a shared
    // PID-derived path — and so cleanup is handled by RAII drop without a
    // race against sibling installs.
    let temp_dir_handle = tempfile::Builder::new()
        .prefix("corkscrew_install_")
        .tempdir()
        .map_err(InstallerError::Io)?;
    let temp_dir = temp_dir_handle.path().to_path_buf();

    info!(
        "Extracting archive {} -> {}",
        archive_path.display(),
        temp_dir.display()
    );
    extract_archive(archive_path, &temp_dir)?;

    // 2. Locate the real mod root inside the extracted tree.
    let data_root = find_data_root(&temp_dir);
    info!("Detected mod data root: {}", data_root.display());

    // 3. Walk the data root and copy files into data_dir.
    fs::create_dir_all(data_dir)?;

    let mut installed_files: Vec<String> = Vec::new();

    for entry in WalkDir::new(&data_root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }

        let abs_src = entry.path();
        let relative = abs_src
            .strip_prefix(&data_root)
            .map_err(|e| InstallerError::Other(e.to_string()))?;

        let dest_path = data_dir.join(relative);

        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::copy(abs_src, &dest_path)?;

        // Store the relative path using forward slashes for consistency.
        let rel_str = relative.to_string_lossy().replace('\\', "/");
        debug!("Installed: {}", rel_str);
        installed_files.push(rel_str);
    }

    info!(
        "Installed {} files into {}",
        installed_files.len(),
        data_dir.display()
    );
    Ok(installed_files)
}

/// Remove previously installed mod files from `data_dir`.
///
/// `installed_files` should be the list of relative paths returned by
/// [`install_mod`].
///
/// After deleting each file the function walks upward and removes any
/// directories that have become empty (up to, but not including, `data_dir`
/// itself).
///
/// Returns the list of files that were actually removed (some may have been
/// deleted or overwritten by another mod in the meantime).
pub fn uninstall_mod_files(data_dir: &Path, installed_files: &[String]) -> Result<Vec<String>> {
    let mut removed: Vec<String> = Vec::new();

    for rel_path_str in installed_files {
        // installed_files comes from the database — an external source per
        // the path-safety invariant. Reject traversal before joining so a
        // tampered/legacy row can never delete outside data_dir.
        if !crate::staging::is_safe_relative_path(rel_path_str) {
            warn!("Uninstall: skipping unsafe relative path: {}", rel_path_str);
            continue;
        }

        let rel_path = Path::new(rel_path_str);
        let full_path = data_dir.join(rel_path);

        // Never delete through a symlink (checked before the remove, TOCTOU).
        if let Ok(meta) = fs::symlink_metadata(&full_path) {
            if meta.file_type().is_symlink() {
                warn!("Uninstall: skipping symlink: {}", full_path.display());
                continue;
            }
        }

        if full_path.exists() {
            fs::remove_file(&full_path)?;
            debug!("Removed: {}", full_path.display());
            removed.push(rel_path_str.clone());

            // Walk upward and prune empty directories, never leaving data_dir.
            let mut current = full_path.parent().map(|p| p.to_path_buf());
            while let Some(dir) = current {
                // Never delete data_dir itself, and never prune outside it.
                if dir == data_dir || !dir.starts_with(data_dir) {
                    break;
                }
                // Only remove if the directory is empty.
                let is_empty = fs::read_dir(&dir)
                    .map(|mut rd| rd.next().is_none())
                    .unwrap_or(false);
                if is_empty {
                    debug!("Removing empty directory: {}", dir.display());
                    fs::remove_dir(&dir)?;
                    current = dir.parent().map(|p| p.to_path_buf());
                } else {
                    break;
                }
            }
        } else {
            warn!("File already missing, skipping: {}", full_path.display());
        }
    }

    info!(
        "Uninstalled {}/{} files from {}",
        removed.len(),
        installed_files.len(),
        data_dir.display()
    );
    Ok(removed)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Arc;

    /// Helper: create a directory tree that mimics a typical Skyrim mod.
    fn create_fake_mod_tree(root: &Path) {
        let meshes = root.join("meshes").join("armor");
        fs::create_dir_all(&meshes).unwrap();
        fs::write(meshes.join("cuirass.nif"), b"fake nif").unwrap();

        let textures = root.join("textures").join("armor");
        fs::create_dir_all(&textures).unwrap();
        fs::write(textures.join("cuirass.dds"), b"fake dds").unwrap();

        fs::write(root.join("mymod.esp"), b"fake esp").unwrap();
    }

    #[test]
    fn test_looks_like_mod_content_positive() {
        let tmp = std::env::temp_dir().join("corkscrew_test_mod_pos");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        create_fake_mod_tree(&tmp);
        assert!(looks_like_mod_content(&tmp));

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_looks_like_mod_content_negative() {
        let tmp = std::env::temp_dir().join("corkscrew_test_mod_neg");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("readme.txt"), b"hello").unwrap();

        assert!(!looks_like_mod_content(&tmp));

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_find_data_root_direct_content() {
        // Root itself has mod files -> should return root.
        let tmp = std::env::temp_dir().join("corkscrew_test_fdr_direct");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        create_fake_mod_tree(&tmp);

        let root = find_data_root(&tmp);
        assert_eq!(root, tmp);

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_find_data_root_single_wrapper() {
        // Archive has one wrapper dir that contains mod content.
        let tmp = std::env::temp_dir().join("corkscrew_test_fdr_wrapper");
        let _ = fs::remove_dir_all(&tmp);
        let inner = tmp.join("SomeMod-v1.0");
        fs::create_dir_all(&inner).unwrap();
        create_fake_mod_tree(&inner);

        let root = find_data_root(&tmp);
        assert_eq!(root, inner);

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_find_data_root_data_subfolder() {
        // Archive has a "Data" child folder alongside other files.
        let tmp = std::env::temp_dir().join("corkscrew_test_fdr_data");
        let _ = fs::remove_dir_all(&tmp);
        let data = tmp.join("Data");
        fs::create_dir_all(&data).unwrap();
        create_fake_mod_tree(&data);
        // Also put a readme at the top level so there are multiple entries.
        fs::write(tmp.join("readme.txt"), b"read me").unwrap();

        let root = find_data_root(&tmp);
        assert_eq!(root, data);

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_find_data_root_single_data_dir() {
        // Archive has exactly one folder named "data" (lowercase).
        let tmp = std::env::temp_dir().join("corkscrew_test_fdr_singledata");
        let _ = fs::remove_dir_all(&tmp);
        let data = tmp.join("data");
        fs::create_dir_all(&data).unwrap();
        create_fake_mod_tree(&data);

        let root = find_data_root(&tmp);
        assert_eq!(root, data);

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_find_data_root_skse_folder() {
        // Archive starts with a known mod folder like SKSE/ — should NOT
        // recurse into it, the parent is the data root.
        // This is the Address Library for SKSE Plugins case:
        //   SKSE/Plugins/versionlib.bin
        let tmp = std::env::temp_dir().join("corkscrew_test_fdr_skse");
        let _ = fs::remove_dir_all(&tmp);
        let skse_plugins = tmp.join("SKSE").join("Plugins");
        fs::create_dir_all(&skse_plugins).unwrap();
        fs::write(skse_plugins.join("versionlib-1-6-640-0.bin"), b"bin").unwrap();

        let root = find_data_root(&tmp);
        // The data root should be tmp (parent of SKSE/), NOT SKSE/Plugins/
        assert_eq!(root, tmp);

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_find_data_root_meshes_folder() {
        // Archive starts with "meshes/" directly — should not recurse.
        let tmp = std::env::temp_dir().join("corkscrew_test_fdr_meshes");
        let _ = fs::remove_dir_all(&tmp);
        let meshes = tmp.join("meshes").join("armor");
        fs::create_dir_all(&meshes).unwrap();
        fs::write(meshes.join("cuirass.nif"), b"nif").unwrap();

        let root = find_data_root(&tmp);
        assert_eq!(root, tmp);

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_find_data_root_bepinex_archive() {
        // Regression: BepInEx-shaped archive starts with `BepInEx/plugins/foo.dll`.
        // Before the Unity/BepInEx folder additions, find_data_root would
        // recurse INTO the single `BepInEx` directory and return
        // `BepInEx/plugins`, stripping the prefix. After the fix, the
        // single top-level `bepinex/` is recognized as a mod folder and the
        // PARENT (extracted_dir) is returned as the data root.
        let tmp = tempfile::tempdir().unwrap();
        let plugins = tmp.path().join("BepInEx").join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        fs::write(plugins.join("MyPlugin.dll"), b"dll").unwrap();

        let root = find_data_root(tmp.path());
        assert_eq!(root, tmp.path());
    }

    #[test]
    fn test_find_data_root_smapi_archive_with_manifest() {
        // SMAPI mod ships a `manifest.json` alongside an assembly DLL.
        // The directory holding the manifest is the data root.
        let tmp = tempfile::tempdir().unwrap();
        let mod_dir = tmp.path().join("MyMod");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(mod_dir.join("manifest.json"), b"{}").unwrap();
        fs::write(mod_dir.join("MyMod.dll"), b"dll").unwrap();

        // With a single wrapper dir containing manifest.json, that wrapper
        // looks like mod content (manifest signal) -> return it.
        let root = find_data_root(tmp.path());
        assert_eq!(root, mod_dir);
    }

    #[test]
    fn test_find_data_root_ue_paks_at_root() {
        // UE archive with .pak files at the top level. The .pak extension
        // is in MOD_FILE_EXTENSIONS so the root looks like mod content.
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("MyMod_P.pak"), b"pak").unwrap();

        let root = find_data_root(tmp.path());
        assert_eq!(root, tmp.path());
    }

    #[test]
    fn test_find_data_root_rimworld_about() {
        // RimWorld mod has `About/About.xml`. The `about` directory name
        // should be recognized.
        let tmp = tempfile::tempdir().unwrap();
        let about = tmp.path().join("About");
        fs::create_dir_all(&about).unwrap();
        fs::write(about.join("About.xml"), b"<xml/>").unwrap();

        let root = find_data_root(tmp.path());
        assert_eq!(root, tmp.path());
    }

    #[test]
    fn test_uninstall_mod_files() {
        let tmp = std::env::temp_dir().join("corkscrew_test_uninstall");
        let _ = fs::remove_dir_all(&tmp);

        // Set up a fake data dir with some files.
        let meshes = tmp.join("meshes").join("armor");
        fs::create_dir_all(&meshes).unwrap();
        fs::write(meshes.join("cuirass.nif"), b"nif").unwrap();
        fs::write(tmp.join("mymod.esp"), b"esp").unwrap();

        let files = vec![
            "meshes/armor/cuirass.nif".to_string(),
            "mymod.esp".to_string(),
        ];

        let removed = uninstall_mod_files(&tmp, &files).unwrap();
        assert_eq!(removed.len(), 2);

        // The meshes/armor directory tree should have been pruned.
        assert!(!tmp.join("meshes").exists());
        assert!(!tmp.join("mymod.esp").exists());

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_uninstall_missing_file() {
        let tmp = std::env::temp_dir().join("corkscrew_test_uninstall_missing");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let files = vec!["does_not_exist.esp".to_string()];
        let removed = uninstall_mod_files(&tmp, &files).unwrap();
        assert!(removed.is_empty());

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_extract_archive_unsupported_format() {
        let tmp = std::env::temp_dir().join("corkscrew_test_unsupported");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let fake_file = tmp.join("archive.cab");
        fs::write(&fake_file, b"not a real cab").unwrap();

        let result = extract_archive(&fake_file, &tmp.join("out"));
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("Unsupported archive format"));

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_extract_archive_not_found() {
        let result = extract_archive(
            Path::new("/tmp/corkscrew_nonexistent_archive.zip"),
            Path::new("/tmp/corkscrew_out"),
        );
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("Archive not found"));
    }

    #[test]
    fn test_is_deploy_junk() {
        // Existing junk still detected
        assert!(is_deploy_junk(Path::new(".DS_Store")));
        assert!(is_deploy_junk(Path::new("__MACOSX/something")));
        assert!(is_deploy_junk(Path::new("Thumbs.db")));

        // FOMOD metadata directories
        assert!(is_deploy_junk(Path::new("fomod/ModuleConfig.xml")));
        assert!(is_deploy_junk(Path::new("fomod/images/preview.png")));
        assert!(is_deploy_junk(Path::new("FoMod/info.xml")));
        assert!(is_deploy_junk(Path::new("__fomod/something")));

        // Loose metadata files
        assert!(is_deploy_junk(Path::new("meta.ini")));
        assert!(is_deploy_junk(Path::new("info.xml")));
        assert!(is_deploy_junk(Path::new("ModuleConfig.xml")));

        // Valid mod content should NOT be junk
        assert!(!is_deploy_junk(Path::new("meshes/armor/cuirass.nif")));
        assert!(!is_deploy_junk(Path::new("textures/diffuse.dds")));
        assert!(!is_deploy_junk(Path::new("mymod.esp")));
        assert!(!is_deploy_junk(Path::new("skse/plugins/engine.dll")));
        assert!(!is_deploy_junk(Path::new("scripts/script.pex")));
    }

    #[test]
    fn test_cp437_to_utf8_ascii() {
        let input = b"hello.txt";
        assert_eq!(cp437_to_utf8(input), "hello.txt");
    }

    #[test]
    fn test_cp437_to_utf8_high_bytes() {
        // 0x81 = 'ü', 0x82 = 'é'
        let input = &[0x68, 0x81, 0x82]; // "hüé"
        assert_eq!(cp437_to_utf8(input), "hüé");
    }

    #[test]
    fn test_detect_bsa_magic() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bsa");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&[0x42, 0x53, 0x41, 0x00, 0, 0, 0, 0]).unwrap();
        assert_eq!(detect_format_from_magic(&path), Some("bsa"));
    }

    #[test]
    fn test_detect_ba2_magic() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.ba2");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&[0x42, 0x54, 0x44, 0x58, 0, 0, 0, 0]).unwrap();
        assert_eq!(detect_format_from_magic(&path), Some("ba2"));
    }

    #[test]
    fn test_extract_zip_skips_symlink_entries() {
        // Regression: a malicious ZIP can plant a symlink entry that points
        // outside the destination dir; subsequent file entries written
        // through the planted name then escape the sandbox. The
        // component-based path-traversal check does NOT catch this because
        // the symlink's stored path itself is benign — the danger is the
        // symlink target.
        //
        // This test crafts a ZIP with one regular file and one symlink
        // entry (built via the zip crate's `add_symlink` helper, which
        // ORs in the S_IFLNK high bits), then asserts the symlink entry
        // is skipped at extraction time and only the regular file is
        // materialized.
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("attack.zip");
        {
            let f = std::fs::File::create(&zip_path).unwrap();
            let mut w = zip::ZipWriter::new(f);

            // Regular file entry
            let opts: SimpleFileOptions = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o644);
            w.start_file("normal.txt", opts).unwrap();
            w.write_all(b"hello").unwrap();

            // Symlink entry — `add_symlink` sets the S_IFLNK high bits in
            // the unix mode field; the symlink target is stored as the
            // entry payload.
            let link_opts: SimpleFileOptions = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o777);
            w.add_symlink("evil_link", "/etc/passwd", link_opts).unwrap();

            w.finish().unwrap();
        }

        let out_dir = dir.path().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();
        let extracted = extract_zip(&zip_path, &out_dir).expect("extract");

        // Only the normal file should have been extracted.
        assert!(
            out_dir.join("normal.txt").exists(),
            "regular file must be extracted"
        );
        assert!(
            !out_dir.join("evil_link").exists(),
            "symlink entry MUST be skipped (path-traversal hardening)"
        );
        // And the returned list should only contain the regular file.
        assert_eq!(extracted.len(), 1, "only one entry should be extracted");
    }

    // -----------------------------------------------------------------------
    // Backslash entry names + traversal (bug #8)
    // -----------------------------------------------------------------------

    /// Entries used by the backslash fixtures: a Windows-separator plugin
    /// path, a backslash traversal attempt, and a normal file.
    const BACKSLASH_ENTRIES: &[(&str, &[u8])] = &[
        ("..\\..\\evil.txt", b"pwned"),
        ("SKSE\\Plugins\\a.dll", b"dll bytes"),
        ("readme.txt", b"hello"),
    ];

    /// Layout: `<tmp>/a/b/out` is the extraction dir, so `..\..\evil.txt`
    /// would land in `<tmp>/a/evil.txt` if traversal were honoured.
    fn backslash_dirs() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("a").join("b").join("out");
        fs::create_dir_all(&out).unwrap();
        (tmp, out)
    }

    /// Asserts the backslash fixture extracted correctly and nothing escaped.
    fn assert_backslash_fixture_extracted(tmp: &Path, out: &Path) {
        let dll = out.join("SKSE").join("Plugins").join("a.dll");
        assert!(dll.is_file(), "a.dll must land at SKSE/Plugins/a.dll");
        assert_eq!(fs::read(&dll).unwrap(), b"dll bytes");
        assert_eq!(fs::read(out.join("readme.txt")).unwrap(), b"hello");

        for escaped in [
            tmp.join("evil.txt"),
            tmp.join("a").join("evil.txt"),
            tmp.join("a").join("b").join("evil.txt"),
        ] {
            assert!(!escaped.exists(), "traversal entry escaped to {}", escaped.display());
        }

        // Nothing anywhere under the temp root may still carry a literal
        // backslash or be named evil.txt, and only the expected files exist
        // besides the archive itself.
        let mut files: Vec<String> = WalkDir::new(tmp)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path().strip_prefix(tmp).unwrap().to_string_lossy().into_owned())
            .filter(|p| !p.ends_with(".zip") && !p.ends_with(".7z"))
            .collect();
        files.sort();
        for f in &files {
            assert!(!f.contains('\\'), "literal backslash left in name: {}", f);
            assert!(!f.ends_with("evil.txt"), "traversal entry written: {}", f);
        }
        assert_eq!(
            files,
            vec![
                "a/b/out/SKSE/Plugins/a.dll".to_string(),
                "a/b/out/readme.txt".to_string()
            ]
        );
    }

    fn write_backslash_zip(path: &Path) {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        let f = fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, data) in BACKSLASH_ENTRIES {
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    /// Solid 7z (one block for every entry) so skipping the unsafe entry
    /// also proves its bytes are drained before the next entry is decoded.
    fn write_backslash_7z(path: &Path) {
        let mut w = sevenz_rust2::ArchiveWriter::create(path).unwrap();
        let entries: Vec<sevenz_rust2::ArchiveEntry> = BACKSLASH_ENTRIES
            .iter()
            .map(|(name, _)| sevenz_rust2::ArchiveEntry::new_file(name))
            .collect();
        let readers: Vec<sevenz_rust2::SourceReader<&[u8]>> = BACKSLASH_ENTRIES
            .iter()
            .map(|(_, data)| sevenz_rust2::SourceReader::new(*data))
            .collect();
        w.push_archive_entries(entries, readers).unwrap();
        w.finish().unwrap();
    }

    #[test]
    fn test_safe_entry_path() {
        assert_eq!(
            safe_entry_path("SKSE\\Plugins\\a.dll"),
            Some(PathBuf::from("SKSE/Plugins/a.dll"))
        );
        assert_eq!(
            safe_entry_path("Data\\SKSE\\Plugins\\PriorityMod.dll"),
            Some(PathBuf::from("Data/SKSE/Plugins/PriorityMod.dll"))
        );
        assert_eq!(safe_entry_path("./a//b/./c.txt"), Some(PathBuf::from("a/b/c.txt")));
        assert_eq!(safe_entry_path("..\\..\\evil.txt"), None);
        assert_eq!(safe_entry_path("a\\..\\..\\evil.txt"), None);
        assert_eq!(safe_entry_path("../evil.txt"), None);
        assert_eq!(safe_entry_path("/etc/passwd"), None);
        assert_eq!(safe_entry_path("\\Windows\\evil.dll"), None);
        assert_eq!(safe_entry_path("C:\\Windows\\evil.dll"), None);
        assert_eq!(safe_entry_path("\\\\?\\C:\\evil"), None);
        assert_eq!(safe_entry_path("a\0b"), None);
        assert_eq!(safe_entry_path("."), None);
        assert_eq!(safe_entry_path(""), None);
        // Dots inside a name are fine.
        assert_eq!(safe_entry_path("my..mod.esp"), Some(PathBuf::from("my..mod.esp")));
    }

    #[test]
    fn test_extract_zip_normalizes_backslash_entries() {
        let (tmp, out) = backslash_dirs();
        let zip_path = tmp.path().join("backslash.zip");
        write_backslash_zip(&zip_path);

        let extracted = extract_archive(&zip_path, &out).expect("extract");
        assert_eq!(extracted.len(), 2);
        assert_backslash_fixture_extracted(tmp.path(), &out);
    }

    #[test]
    fn test_extract_zip_with_progress_normalizes_backslash_entries() {
        let (tmp, out) = backslash_dirs();
        let zip_path = tmp.path().join("backslash.zip");
        write_backslash_zip(&zip_path);

        let last = Arc::new(std::sync::Mutex::new((0u64, 0u64)));
        let last_c = Arc::clone(&last);
        let cb = move |done: u64, total: u64| *last_c.lock().unwrap() = (done, total);
        extract_archive_with_progress(&zip_path, &out, &cb).expect("extract");
        assert_eq!(*last.lock().unwrap(), (2, 2));
        assert_backslash_fixture_extracted(tmp.path(), &out);
    }

    #[test]
    fn test_extract_7z_rust_normalizes_backslash_entries() {
        let (tmp, out) = backslash_dirs();
        let archive = tmp.path().join("backslash.7z");
        write_backslash_7z(&archive);

        // Call the pure-Rust path directly: `extract_7z` would prefer a
        // native binary when one is installed.
        let extracted = extract_7z_rust(&archive, &out, &never_cancelled).expect("extract");
        assert_eq!(extracted.len(), 2);
        assert_backslash_fixture_extracted(tmp.path(), &out);
    }

    #[test]
    fn test_extract_7z_native_normalizes_backslash_entries() {
        let Some(bin) = find_native_7z() else {
            eprintln!("skipping: no native 7z binary installed");
            return;
        };
        let (tmp, out) = backslash_dirs();
        let archive = tmp.path().join("backslash.7z");
        write_backslash_7z(&archive);

        let extracted =
            extract_7z_native(&bin, &archive, &out, &never_cancelled).expect("extract");
        assert_eq!(extracted.len(), 2);
        assert_backslash_fixture_extracted(tmp.path(), &out);
    }

    #[test]
    fn test_fix_backslash_entry_names() {
        // Simulates what a native extractor leaves behind on Unix: files
        // whose single name component contains literal backslashes.
        let (tmp, out) = backslash_dirs();
        fs::write(out.join("SKSE\\Plugins\\a.dll"), b"dll bytes").unwrap();
        fs::write(out.join("..\\..\\evil.txt"), b"pwned").unwrap();
        fs::write(out.join("readme.txt"), b"hello").unwrap();
        // A directory with a backslash name holding a file.
        fs::create_dir_all(out.join("Data\\Scripts")).unwrap();
        fs::write(out.join("Data\\Scripts").join("x.pex"), b"pex").unwrap();

        fix_backslash_entry_names(&out);

        assert_eq!(fs::read(out.join("Data/Scripts/x.pex")).unwrap(), b"pex");
        assert!(!out.join("Data\\Scripts").exists(), "emptied backslash dir removed");
        fs::remove_dir_all(out.join("Data")).unwrap();
        assert_backslash_fixture_extracted(tmp.path(), &out);
    }

    // -----------------------------------------------------------------------
    // Cancellation (bug #2)
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_cancelled_before_start() {
        let (tmp, out) = backslash_dirs();
        let zip_path = tmp.path().join("backslash.zip");
        write_backslash_zip(&zip_path);
        let result = extract_archive_cancellable(&zip_path, &out, &|| true);
        assert!(matches!(result, Err(InstallerError::Cancelled)));
        assert!(!out.join("readme.txt").exists());
    }

    #[test]
    fn test_extract_7z_rust_cancelled_mid_archive() {
        let (tmp, out) = backslash_dirs();
        let archive = tmp.path().join("backslash.7z");
        write_backslash_7z(&archive);
        // Allow the first couple of checks, then cancel.
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, Ordering::SeqCst) >= 2;
        let result = extract_7z_rust(&archive, &out, &cancel);
        assert!(matches!(result, Err(InstallerError::Cancelled)), "{:?}", result.err());
    }

    #[test]
    fn test_extract_7z_native_cancelled() {
        let Some(bin) = find_native_7z() else {
            eprintln!("skipping: no native 7z binary installed");
            return;
        };
        let (tmp, out) = backslash_dirs();
        let archive = tmp.path().join("backslash.7z");
        write_backslash_7z(&archive);
        let result = extract_7z_native(&bin, &archive, &out, &|| true);
        // The child may finish this tiny archive before the first poll;
        // either outcome is fine, but a cancel must never fall back to the
        // pure-Rust decoder (extract_7z) and must report Cancelled.
        if let Err(e) = result {
            assert!(matches!(e, InstallerError::Cancelled), "{:?}", e);
        }
        assert!(matches!(
            extract_7z(&archive, &out, &|| true),
            Err(InstallerError::Cancelled) | Ok(_)
        ));
    }

    // -----------------------------------------------------------------------
    // FOMOD data root (bug #15)
    // -----------------------------------------------------------------------

    /// MCM-Helper-shaped FOMOD archive: shared `Data/` plus option folders.
    fn create_mcm_helper_tree(root: &Path) {
        let files: &[&str] = &[
            "Data/Interface/Translations/MCMHelper_english.txt",
            "Data/Interface/Translations/MCMHelper_french.txt",
            "Data/MCM/Config/MCMHelper/config.json",
            "Data/SKSE/Plugins/MCMHelper/settings.ini",
            "ESP/MCMHelper.esp",
            "ESL/MCMHelper.esp",
            "BSA/MCMHelper.bsa",
            "Loose/Scripts/MCM_ConfigBase.pex",
            "Loose/Interface/Translations/extra.txt",
            "SkyrimSE/SKSE/Plugins/MCMHelper.dll",
            "SkyrimVR/SKSE/Plugins/MCMHelper.dll",
            "fomod/ModuleConfig.xml",
            "fomod/info.xml",
        ];
        for f in files {
            let p = root.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, f.as_bytes()).unwrap();
        }
    }

    #[test]
    fn test_find_data_root_prefers_fomod_root_over_data_child() {
        let tmp = tempfile::tempdir().unwrap();
        create_mcm_helper_tree(tmp.path());
        assert_eq!(find_data_root(tmp.path()), tmp.path());
    }

    #[test]
    fn test_find_data_root_fomod_root_inside_wrapper() {
        let tmp = tempfile::tempdir().unwrap();
        let wrapper = tmp.path().join("MCM Helper 1.6.3");
        create_mcm_helper_tree(&wrapper);
        assert_eq!(find_data_root(tmp.path()), wrapper);
    }

    #[test]
    fn test_find_data_root_fomod_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("FOMod")).unwrap();
        fs::write(tmp.path().join("FOMod").join("moduleconfig.XML"), b"<config/>").unwrap();
        fs::create_dir_all(tmp.path().join("Data")).unwrap();
        fs::write(tmp.path().join("Data").join("a.esp"), b"esp").unwrap();
        assert_eq!(find_data_root(tmp.path()), tmp.path());
    }

    #[test]
    fn test_find_data_root_info_xml_only_fomod_still_uses_data() {
        // A fomod/ folder with only info.xml (no installer) must not change
        // the Data/ selection.
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("fomod")).unwrap();
        fs::write(tmp.path().join("fomod").join("info.xml"), b"<fomod/>").unwrap();
        let data = tmp.path().join("Data");
        create_fake_mod_tree(&data);
        assert_eq!(find_data_root(tmp.path()), data);
    }

    #[test]
    fn test_find_data_root_skse_style_data_with_root_exe() {
        // SKSE layout: loader exe + DLLs beside Data/ — Data/ is still the
        // data root (root files are preserved separately by staging). No
        // fomod/, so the FOMOD rule must not change this.
        let tmp = tempfile::tempdir().unwrap();
        let wrapper = tmp.path().to_path_buf();
        let data = wrapper.join("Data");
        fs::create_dir_all(data.join("Scripts")).unwrap();
        fs::write(data.join("Scripts").join("skse.pex"), b"pex").unwrap();
        fs::write(wrapper.join("skse64_loader.exe"), b"exe").unwrap();
        fs::write(wrapper.join("skse64_1_6_1170.dll"), b"dll").unwrap();
        assert_eq!(find_data_root(tmp.path()), data);
    }
}
