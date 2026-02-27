use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use log::{debug, info, warn};
use rayon::prelude::*;
use thiserror::Error;
use walkdir::WalkDir;
use zip::ZipArchive;

/// Callback type for reporting extraction progress: (files_done, files_total).
pub type ExtractProgressCb = dyn Fn(u64, u64) + Send + Sync;

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

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, InstallerError>;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// File extensions that are characteristic of Skyrim / Bethesda mods.
const MOD_FILE_EXTENSIONS: &[&str] = &[
    "esp", "esm", "esl", // plugin files
    "bsa", "ba2", // archives
    "nif", // meshes
    "dds", "tga", // textures
    "hkx", // animations
    "pex", // compiled Papyrus scripts
    "seq", // sequence files
    "swf", // UI files
    "fuz", // voice / lip-sync
    "dll", // SKSE plugins
    "bin", // SKSE address library data
    "ini", // configuration files
    "json", // mod config / MCM settings
];

/// Directory names that are characteristic of Skyrim / Bethesda mod content.
const MOD_FOLDER_NAMES: &[&str] = &[
    "meshes",
    "textures",
    "scripts",
    "interface",
    "sound",
    "skse",
];

// ---------------------------------------------------------------------------
// Archive extraction
// ---------------------------------------------------------------------------

/// Extract an archive into `dest_dir`, creating the directory if it does not
/// exist.  Returns the list of files that were extracted (absolute paths).
///
/// Supported formats (matched by file extension):
/// - `.zip`
/// - `.7z`
pub fn extract_archive(archive_path: &Path, dest_dir: &Path) -> Result<Vec<PathBuf>> {
    if !archive_path.exists() {
        return Err(InstallerError::ArchiveNotFound(archive_path.to_path_buf()));
    }

    fs::create_dir_all(dest_dir)?;

    // Check compound extensions first (tar.gz, tar.xz, tar.bz2)
    let name_lower = archive_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();

    if name_lower.ends_with(".tar.gz") || name_lower.ends_with(".tgz") {
        return extract_tar_gz(archive_path, dest_dir);
    }
    if name_lower.ends_with(".tar.xz") || name_lower.ends_with(".txz") {
        return extract_tar_xz(archive_path, dest_dir);
    }
    if name_lower.ends_with(".tar.bz2") || name_lower.ends_with(".tbz2") {
        return extract_tar_bz2(archive_path, dest_dir);
    }

    let ext = archive_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "zip" => extract_zip(archive_path, dest_dir),
        "7z" => extract_7z(archive_path, dest_dir),
        "rar" => extract_rar(archive_path, dest_dir),
        other => Err(InstallerError::UnsupportedFormat(other.to_string())),
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

    let name_lower = archive_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();

    // Tar variants — no per-file progress, just bookend
    if name_lower.ends_with(".tar.gz") || name_lower.ends_with(".tgz") {
        progress(0, 1);
        let result = extract_tar_gz(archive_path, dest_dir);
        if let Ok(ref files) = result {
            progress(files.len() as u64, files.len() as u64);
        }
        return result;
    }
    if name_lower.ends_with(".tar.xz") || name_lower.ends_with(".txz") {
        progress(0, 1);
        let result = extract_tar_xz(archive_path, dest_dir);
        if let Ok(ref files) = result {
            progress(files.len() as u64, files.len() as u64);
        }
        return result;
    }
    if name_lower.ends_with(".tar.bz2") || name_lower.ends_with(".tbz2") {
        progress(0, 1);
        let result = extract_tar_bz2(archive_path, dest_dir);
        if let Ok(ref files) = result {
            progress(files.len() as u64, files.len() as u64);
        }
        return result;
    }

    let ext = archive_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "zip" => extract_zip_with_progress(archive_path, dest_dir, progress),
        "7z" => {
            progress(0, 1);
            let result = extract_7z(archive_path, dest_dir);
            if let Ok(ref files) = result {
                progress(files.len() as u64, files.len() as u64);
            }
            result
        }
        "rar" => {
            progress(0, 1);
            let result = extract_rar(archive_path, dest_dir);
            if let Ok(ref files) = result {
                progress(files.len() as u64, files.len() as u64);
            }
            result
        }
        other => Err(InstallerError::UnsupportedFormat(other.to_string())),
    }
}

/// Extract a `.zip` archive using the `zip` crate with parallel file extraction.
///
/// Uses a two-pass approach:
/// 1. Sequential scan: read central directory, collect file entries, create directories
/// 2. Parallel extraction via rayon: each thread opens its own ZipArchive handle
///    and extracts assigned entries independently (ZIP supports random access)
///
/// Rayon's work-stealing pool automatically balances: when many archives extract
/// concurrently each gets ~1 thread; when only the last few remain, they get all cores.
fn extract_zip(archive_path: &Path, dest_dir: &Path) -> Result<Vec<PathBuf>> {
    let file = io::BufReader::with_capacity(EXTRACT_BUF_SIZE, fs::File::open(archive_path)?);
    let mut archive = ZipArchive::new(file)?;

    // Pass 1: collect file entries and create all directories up front
    let mut file_entries: Vec<(usize, PathBuf)> = Vec::new();

    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        let relative = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => {
                warn!("Skipping ZIP entry with unsafe path");
                continue;
            }
        };
        if relative
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            warn!(
                "Skipping ZIP path traversal attempt: {}",
                relative.display()
            );
            continue;
        }
        let out_path = dest_dir.join(&relative);

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

    // Pass 2: parallel extraction — partition entries into chunks so each rayon
    // thread opens ONE ZipArchive handle and extracts all its entries, avoiding
    // the overhead of re-parsing the central directory per file.
    let num_threads = rayon::current_num_threads().max(1);
    let archive_path_owned = archive_path.to_path_buf();
    let chunks: Vec<&[(usize, PathBuf)]> = file_entries.chunks(
        (file_entries.len() / num_threads).max(1),
    ).collect();

    let results: Vec<Vec<std::result::Result<PathBuf, String>>> = chunks
        .par_iter()
        .map(|chunk| {
            // Open one archive handle per thread chunk
            let f = match fs::File::open(&archive_path_owned) {
                Ok(file) => io::BufReader::with_capacity(EXTRACT_BUF_SIZE, file),
                Err(e) => {
                    return chunk.iter().map(|_| Err(format!("Open failed: {}", e))).collect();
                }
            };
            let mut arch = match ZipArchive::new(f) {
                Ok(a) => a,
                Err(e) => {
                    return chunk.iter().map(|_| Err(format!("ZIP parse failed: {}", e))).collect();
                }
            };
            chunk.iter().map(|(idx, out_path)| {
                let mut entry = arch.by_index(*idx).map_err(|e| e.to_string())?;
                let mut out_file = io::BufWriter::with_capacity(
                    EXTRACT_BUF_SIZE,
                    fs::File::create(out_path).map_err(|e| e.to_string())?,
                );
                io::copy(&mut entry, &mut out_file).map_err(|e| e.to_string())?;
                Ok(out_path.clone())
            }).collect()
        })
        .collect();

    let mut extracted = Vec::with_capacity(file_entries.len());
    for chunk_results in results {
        for r in chunk_results {
            match r {
                Ok(p) => extracted.push(p),
                Err(e) => warn!("Failed to extract ZIP entry: {}", e),
            }
        }
    }

    info!(
        "Extracted {} files from ZIP: {}",
        extracted.len(),
        archive_path.display()
    );
    Ok(extracted)
}

/// Like [`extract_zip`] but reports per-file progress via a shared atomic
/// counter and a throttled callback.
fn extract_zip_with_progress(
    archive_path: &Path,
    dest_dir: &Path,
    progress: &ExtractProgressCb,
) -> Result<Vec<PathBuf>> {
    let file = io::BufReader::with_capacity(EXTRACT_BUF_SIZE, fs::File::open(archive_path)?);
    let mut archive = ZipArchive::new(file)?;

    // Pass 1: collect file entries and create directories
    let mut file_entries: Vec<(usize, PathBuf)> = Vec::new();

    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        let relative = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => {
                warn!("Skipping ZIP entry with unsafe path");
                continue;
            }
        };
        if relative
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            warn!(
                "Skipping ZIP path traversal attempt: {}",
                relative.display()
            );
            continue;
        }
        let out_path = dest_dir.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            file_entries.push((i, out_path));
        }
    }

    drop(archive);

    let total_files = file_entries.len() as u64;
    progress(0, total_files);

    // Shared atomic counter for cross-thread progress tracking
    let files_done = Arc::new(AtomicU64::new(0));

    let num_threads = rayon::current_num_threads().max(1);
    let archive_path_owned = archive_path.to_path_buf();
    let chunks: Vec<&[(usize, PathBuf)]> =
        file_entries.chunks((file_entries.len() / num_threads).max(1)).collect();

    let results: Vec<Vec<std::result::Result<PathBuf, String>>> = chunks
        .par_iter()
        .map(|chunk| {
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
                    let mut entry = arch.by_index(*idx).map_err(|e| e.to_string())?;
                    let mut out_file = io::BufWriter::with_capacity(
                        EXTRACT_BUF_SIZE,
                        fs::File::create(out_path).map_err(|e| e.to_string())?,
                    );
                    io::copy(&mut entry, &mut out_file).map_err(|e| e.to_string())?;

                    let done = files_done.fetch_add(1, Ordering::Relaxed) + 1;
                    // Throttle progress callbacks — emit every ~2% or every 10 files
                    let interval = (total_files / 50).max(10).min(100);
                    if done % interval == 0 || done == total_files {
                        progress(done, total_files);
                    }

                    Ok(out_path.clone())
                })
                .collect()
        })
        .collect();

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
    progress(extracted.len() as u64, total_files);

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
    let prefixes: &[&str] = &[
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
    ];

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
fn extract_7z(archive_path: &Path, dest_dir: &Path) -> Result<Vec<PathBuf>> {
    if let Some(bin) = find_native_7z() {
        match extract_7z_native(&bin, archive_path, dest_dir) {
            Ok(files) => return Ok(files),
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

    extract_7z_rust(archive_path, dest_dir)
}

/// Extract using the native `7z` / `7zz` command-line tool.
fn extract_7z_native(
    bin: &Path,
    archive_path: &Path,
    dest_dir: &Path,
) -> Result<Vec<PathBuf>> {
    info!(
        "Extracting 7z with native binary {}: {} -> {}",
        bin.display(),
        archive_path.display(),
        dest_dir.display()
    );

    let output = Command::new(bin)
        .arg("x")                                         // eXtract with full paths
        .arg(archive_path)
        .arg(format!("-o{}", dest_dir.display()))         // output directory
        .arg("-y")                                        // assume Yes to all prompts
        .arg("-bso0")                                     // suppress normal stdout
        .arg("-bsp0")                                     // suppress progress stdout
        .output()
        .map_err(|e| {
            InstallerError::SevenZ(format!("Failed to run {}: {}", bin.display(), e))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(InstallerError::SevenZ(format!(
            "7z exited with {}: {}",
            output.status,
            stderr.trim()
        )));
    }

    // Walk destination to collect extracted files (same as Rust path)
    collect_extracted_files(dest_dir)
}

/// Extract using the pure-Rust `sevenz-rust2` crate (slow fallback).
fn extract_7z_rust(archive_path: &Path, dest_dir: &Path) -> Result<Vec<PathBuf>> {
    info!(
        "Extracting 7z with pure-Rust decoder: {}",
        archive_path.display()
    );

    sevenz_rust2::decompress_file(archive_path, dest_dir).map_err(|e| {
        InstallerError::SevenZ(format!(
            "Failed to extract 7z {}: {}",
            archive_path.display(),
            e
        ))
    })?;

    collect_extracted_files(dest_dir)
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
                extracted.push(path);
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
fn extract_rar(archive_path: &Path, dest_dir: &Path) -> Result<Vec<PathBuf>> {
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
        let entry = header.entry();

        // Reject entries with ".." components (path traversal)
        if entry
            .filename
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            warn!(
                "Skipping RAR entry with path traversal: {}",
                entry.filename.display()
            );
            let next = header
                .skip()
                .map_err(|e| InstallerError::Rar(e.to_string()))?;
            cursor = Some(
                next.read_header()
                    .map_err(|e| InstallerError::Rar(e.to_string()))?,
            );
            continue;
        }

        let out_path = dest_dir.join(&entry.filename);

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
                    warn!("Removing RAR entry outside destination: {}", canonical.display());
                    let _ = fs::remove_file(&out_path);
                }
            } else {
                extracted.push(out_path);
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
fn extract_tar_gz(archive_path: &Path, dest_dir: &Path) -> Result<Vec<PathBuf>> {
    let file = io::BufReader::with_capacity(EXTRACT_BUF_SIZE, fs::File::open(archive_path)?);
    let decoder = flate2::read::GzDecoder::new(file);
    extract_tar(decoder, archive_path, dest_dir)
}

/// Extract a `.tar.xz` / `.txz` archive.
fn extract_tar_xz(archive_path: &Path, dest_dir: &Path) -> Result<Vec<PathBuf>> {
    let file = io::BufReader::with_capacity(EXTRACT_BUF_SIZE, fs::File::open(archive_path)?);
    let decoder = xz2::read::XzDecoder::new(file);
    extract_tar(decoder, archive_path, dest_dir)
}

/// Extract a `.tar.bz2` / `.tbz2` archive.
fn extract_tar_bz2(archive_path: &Path, dest_dir: &Path) -> Result<Vec<PathBuf>> {
    let file = io::BufReader::with_capacity(EXTRACT_BUF_SIZE, fs::File::open(archive_path)?);
    let decoder = bzip2::read::BzDecoder::new(file);
    extract_tar(decoder, archive_path, dest_dir)
}

/// Shared tar extraction logic for any decompressed reader.
/// TAR is a stream format so parallelism isn't possible, but buffered I/O helps.
fn extract_tar<R: io::Read>(
    reader: R,
    archive_path: &Path,
    dest_dir: &Path,
) -> Result<Vec<PathBuf>> {
    let buffered = io::BufReader::with_capacity(EXTRACT_BUF_SIZE, reader);
    let mut archive = tar::Archive::new(buffered);
    let mut extracted = Vec::new();

    for entry_result in archive
        .entries()
        .map_err(|e| InstallerError::Tar(e.to_string()))?
    {
        let mut entry = entry_result.map_err(|e| InstallerError::Tar(e.to_string()))?;
        let rel_path = entry
            .path()
            .map_err(|e| InstallerError::Tar(e.to_string()))?
            .into_owned();

        let out_path = dest_dir.join(&rel_path);

        // Path traversal check
        if !out_path.starts_with(dest_dir) {
            warn!(
                "Skipping tar entry with path traversal: {}",
                rel_path.display()
            );
            continue;
        }

        // Reject symlinks to prevent symlink-based escape attacks
        if entry.header().entry_type().is_symlink() {
            warn!("Skipping symlink in tar archive: {}", rel_path.display());
            continue;
        }

        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&out_path)?;
        } else if entry.header().entry_type().is_file() {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out_file = io::BufWriter::with_capacity(
                EXTRACT_BUF_SIZE,
                fs::File::create(&out_path)?,
            );
            io::copy(&mut entry, &mut out_file)?;
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

/// Heuristic: does `directory` look like it already contains Bethesda mod
/// content (plugin files, BSAs, or well-known sub-folders)?
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
    }

    false
}

/// Locate the actual root of mod content inside `extracted_dir`.
///
/// Many archives nest content in a single wrapper directory or include an
/// explicit `Data` folder.  This function walks down into that structure so
/// the caller can copy only the relevant files.
///
/// Rules (evaluated in order):
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

            // 1b – looks like mod content
            if looks_like_mod_content(&entry_path) {
                debug!(
                    "find_data_root: single dir looks like mod content -> {}",
                    entry_path.display()
                );
                return entry_path;
            }

            // 1c – if the single directory IS a recognized mod folder name
            // (e.g. "skse", "meshes", "textures"), the current directory is the
            // data root — do NOT recurse into it or we'll strip the folder prefix.
            if MOD_FOLDER_NAMES.contains(&name.as_str()) {
                debug!(
                    "find_data_root: single dir '{}' is a known mod folder -> {}",
                    name,
                    dir.display()
                );
                return dir.to_path_buf();
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
    // 1. Extract into a temp directory.
    let temp_dir = std::env::temp_dir().join(format!("corkscrew_install_{}", std::process::id()));
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir)?;
    }
    fs::create_dir_all(&temp_dir)?;

    // Make sure we clean up the temp dir even on early return.
    let _cleanup = TempDirGuard(temp_dir.clone());

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
        let rel_path = Path::new(rel_path_str);
        let full_path = data_dir.join(rel_path);

        if full_path.exists() {
            fs::remove_file(&full_path)?;
            debug!("Removed: {}", full_path.display());
            removed.push(rel_path_str.clone());

            // Walk upward and prune empty directories.
            let mut current = full_path.parent().map(|p| p.to_path_buf());
            while let Some(dir) = current {
                // Never delete data_dir itself.
                if dir == data_dir {
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

/// RAII guard that removes a temporary directory when dropped.
struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if self.0.exists() {
            if let Err(e) = fs::remove_dir_all(&self.0) {
                warn!("Failed to clean up temp dir {}: {}", self.0.display(), e);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
}
