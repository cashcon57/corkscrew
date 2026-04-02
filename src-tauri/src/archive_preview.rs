//! Archive content browsing and DDS texture thumbnail previews.
//!
//! Supports BSA (TES3/TES4), BA2 (FO4/Starfield), ZIP, and 7z archives.
//! Reads archive tables of contents without full extraction and can extract
//! individual DDS textures, converting them to base64-encoded PNG thumbnails.

use anyhow::{bail, Context, Result};
use ba2::ByteSlice;
use base64::Engine;
use image::imageops::FilterType;
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A single file or directory entry within an archive.
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveEntry {
    /// File or directory name (leaf name only, e.g. "texture.dds").
    pub name: String,
    /// Full relative path within the archive (forward-slash separated).
    pub path: String,
    /// Whether this entry represents a directory.
    pub is_dir: bool,
    /// Uncompressed file size in bytes (0 for directories).
    pub file_size: u64,
    /// Compressed size in bytes (0 if not compressed or unknown, 0 for dirs).
    pub compressed_size: u64,
}

/// Summary of an archive's contents.
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveContents {
    /// Archive format identifier: "bsa", "ba2", "zip", "7z".
    pub archive_type: String,
    /// Total number of files (excluding synthetic directory entries).
    pub total_files: usize,
    /// Sum of uncompressed file sizes.
    pub total_size: u64,
    /// Flat list of all entries (files and directories).
    pub entries: Vec<ArchiveEntry>,
}

/// A node in a hierarchical file tree built from flat archive entries.
#[derive(Debug, Clone, Serialize)]
pub struct FileTreeNode {
    /// Display name (leaf component).
    pub name: String,
    /// Full path within the archive.
    pub path: String,
    /// Whether this node is a directory.
    pub is_dir: bool,
    /// Children (non-empty only for directories).
    pub children: Vec<FileTreeNode>,
    /// File size in bytes (0 for directories).
    pub file_size: u64,
}

// ---------------------------------------------------------------------------
// Archive listing
// ---------------------------------------------------------------------------

/// Read the table of contents of an archive without extracting file data.
///
/// Supports `.bsa`, `.ba2`, `.zip`, and `.7z` archives. The format is
/// detected by file extension (case-insensitive).
pub fn list_archive_contents(archive_path: &Path) -> Result<ArchiveContents> {
    let ext = archive_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "bsa" => list_bsa_contents(archive_path),
        "ba2" => list_ba2_contents(archive_path),
        "zip" => list_zip_contents(archive_path),
        "7z" => list_7z_contents(archive_path),
        _ => bail!("Unsupported archive format: .{ext}"),
    }
}

// -- BSA (TES3 / TES4) -----------------------------------------------------

fn list_bsa_contents(archive_path: &Path) -> Result<ArchiveContents> {
    use ba2::{guess_format, FileFormat};

    let mut file = std::fs::File::open(archive_path)
        .with_context(|| format!("Failed to open BSA: {}", archive_path.display()))?;
    let format = guess_format(&mut file);

    match format {
        Some(FileFormat::TES3) => list_bsa_tes3(archive_path),
        Some(FileFormat::TES4) => list_bsa_tes4(archive_path),
        Some(FileFormat::FO4) => {
            // Mis-labeled .bsa that is actually a BA2 — handle gracefully
            list_ba2_contents(archive_path)
        }
        None => bail!(
            "Unrecognized BSA format in {}",
            archive_path.display()
        ),
    }
}

fn list_bsa_tes3(archive_path: &Path) -> Result<ArchiveContents> {
    use ba2::prelude::*;
    use ba2::tes3::Archive;

    let archive: Archive<'static> = Archive::read(archive_path)
        .map_err(|e| anyhow::anyhow!("Failed to read TES3 BSA: {e}"))?;

    let mut entries = Vec::with_capacity(archive.len());
    let mut total_size: u64 = 0;

    for (key, file) in archive.iter() {
        let raw_path = key.name().to_str_lossy().to_string();
        let normalized = normalize_archive_path(&raw_path);
        let name = leaf_name(&normalized);
        let size = file.len() as u64;
        total_size += size;

        entries.push(ArchiveEntry {
            name,
            path: normalized,
            is_dir: false,
            file_size: size,
            compressed_size: 0,
        });
    }

    Ok(ArchiveContents {
        archive_type: "bsa".to_string(),
        total_files: entries.len(),
        total_size,
        entries,
    })
}

fn list_bsa_tes4(archive_path: &Path) -> Result<ArchiveContents> {
    use ba2::prelude::*;
    use ba2::tes4::Archive;

    let (archive, _meta): (Archive<'static>, _) = Archive::read(archive_path)
        .map_err(|e| anyhow::anyhow!("Failed to read TES4 BSA: {e}"))?;

    let mut entries = Vec::new();
    let mut total_size: u64 = 0;

    for (dir_key, directory) in archive.iter() {
        let dir_name = dir_key.name().to_str_lossy().to_string();
        let dir_path = normalize_archive_path(&dir_name);

        for (file_key, file) in directory.iter() {
            let file_name = file_key.name().to_str_lossy().to_string();
            let full_path = if dir_path.is_empty() {
                normalize_archive_path(&file_name)
            } else {
                format!("{}/{}", dir_path, normalize_archive_path(&file_name))
            };
            let name = leaf_name(&full_path);
            let size = file.len() as u64;
            total_size += size;

            entries.push(ArchiveEntry {
                name,
                path: full_path,
                is_dir: false,
                file_size: size,
                compressed_size: if file.is_compressed() {
                    file.len() as u64
                } else {
                    0
                },
            });
        }
    }

    Ok(ArchiveContents {
        archive_type: "bsa".to_string(),
        total_files: entries.len(),
        total_size,
        entries,
    })
}

// -- BA2 (FO4 / Starfield) -------------------------------------------------

fn list_ba2_contents(archive_path: &Path) -> Result<ArchiveContents> {
    use ba2::fo4::Archive;
    use ba2::prelude::*;

    let (archive, _meta): (Archive<'static>, _) = Archive::read(archive_path)
        .map_err(|e| anyhow::anyhow!("Failed to read BA2: {e}"))?;

    let mut entries = Vec::with_capacity(archive.len());
    let mut total_size: u64 = 0;

    for (key, file) in archive.iter() {
        let raw_path = key.name().to_str_lossy().to_string();
        let normalized = normalize_archive_path(&raw_path);
        let name = leaf_name(&normalized);

        // Sum chunk decompressed sizes for total uncompressed size estimate.
        // For compressed chunks, decompressed_len gives the original size;
        // for uncompressed chunks it is None, so fall back to len().
        let mut file_size: u64 = 0;
        let mut compressed_size: u64 = 0;
        for chunk in file.iter() {
            let decompressed = chunk
                .decompressed_len()
                .unwrap_or(chunk.len());
            file_size += decompressed as u64;
            compressed_size += chunk.len() as u64;
        }
        total_size += file_size;

        entries.push(ArchiveEntry {
            name,
            path: normalized,
            is_dir: false,
            file_size,
            compressed_size: if compressed_size < file_size {
                compressed_size
            } else {
                0
            },
        });
    }

    Ok(ArchiveContents {
        archive_type: "ba2".to_string(),
        total_files: entries.len(),
        total_size,
        entries,
    })
}

// -- ZIP --------------------------------------------------------------------

fn list_zip_contents(archive_path: &Path) -> Result<ArchiveContents> {
    let file = std::fs::File::open(archive_path)
        .with_context(|| format!("Failed to open ZIP: {}", archive_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("Failed to parse ZIP: {}", archive_path.display()))?;

    let mut entries = Vec::with_capacity(archive.len());
    let mut total_size: u64 = 0;

    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i)?;
        let raw_path = entry.name().to_string();
        let is_dir = entry.is_dir();
        let normalized = normalize_archive_path(&raw_path);
        let name = leaf_name(&normalized);

        if is_dir {
            entries.push(ArchiveEntry {
                name,
                path: normalized,
                is_dir: true,
                file_size: 0,
                compressed_size: 0,
            });
        } else {
            let file_size = entry.size();
            let compressed_size = entry.compressed_size();
            total_size += file_size;

            entries.push(ArchiveEntry {
                name,
                path: normalized,
                is_dir: false,
                file_size,
                compressed_size: if compressed_size < file_size {
                    compressed_size
                } else {
                    0
                },
            });
        }
    }

    Ok(ArchiveContents {
        archive_type: "zip".to_string(),
        total_files: entries.iter().filter(|e| !e.is_dir).count(),
        total_size,
        entries,
    })
}

// -- 7z ---------------------------------------------------------------------

fn list_7z_contents(archive_path: &Path) -> Result<ArchiveContents> {
    let mut entries = Vec::new();
    let mut total_size: u64 = 0;

    let file = std::fs::File::open(archive_path)
        .with_context(|| format!("Failed to open 7z: {}", archive_path.display()))?;

    sevenz_rust2::decompress_with_extract_fn(
        file,
        ".",
        |entry, _reader: &mut dyn Read, _dest| {
            let raw_path = entry.name().to_string();
            let is_dir = entry.is_directory();
            let normalized = normalize_archive_path(&raw_path);
            let name = leaf_name(&normalized);
            let file_size = entry.size();

            if !is_dir {
                total_size += file_size;
            }

            entries.push(ArchiveEntry {
                name,
                path: normalized,
                is_dir,
                file_size: if is_dir { 0 } else { file_size },
                compressed_size: 0, // 7z crate doesn't expose per-file compressed size
            });

            Ok(true) // continue iteration, don't actually extract
        },
    )
    .map_err(|e| anyhow::anyhow!("Failed to read 7z {}: {e}", archive_path.display()))?;

    let total_files = entries.iter().filter(|e| !e.is_dir).count();

    Ok(ArchiveContents {
        archive_type: "7z".to_string(),
        total_files,
        total_size,
        entries,
    })
}

// ---------------------------------------------------------------------------
// Thumbnail extraction
// ---------------------------------------------------------------------------

/// Extract a single DDS file from an archive, convert to PNG, resize to
/// `max_size` pixels on the longest edge, and return as a base64 string.
///
/// Returns `Ok(base64_png)` on success.
pub fn extract_thumbnail(
    archive_path: &Path,
    internal_path: &str,
    max_size: u32,
) -> Result<String> {
    let ext = archive_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let raw_data = match ext.as_str() {
        "bsa" => extract_file_from_bsa(archive_path, internal_path)?,
        "ba2" => extract_file_from_ba2(archive_path, internal_path)?,
        "zip" => extract_file_from_zip(archive_path, internal_path)?,
        "7z" => extract_file_from_7z(archive_path, internal_path)?,
        _ => bail!("Unsupported archive format for thumbnail: .{ext}"),
    };

    dds_bytes_to_base64_png(&raw_data, max_size)
}

/// Convert raw DDS bytes into a resized base64-encoded PNG string.
fn dds_bytes_to_base64_png(dds_bytes: &[u8], max_size: u32) -> Result<String> {
    let dds = ddsfile::Dds::read(&mut Cursor::new(dds_bytes))
        .context("Failed to parse DDS data")?;

    let rgba = image_dds::image_from_dds(&dds, 0)
        .context("Failed to decode DDS to RGBA")?;

    let (w, h) = (rgba.width(), rgba.height());
    let longest = w.max(h);
    let thumbnail = if longest > max_size {
        image::imageops::resize(&rgba, max_size, max_size, FilterType::Lanczos3)
    } else {
        rgba
    };

    let mut png_buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png_buf);
    image::ImageEncoder::write_image(
        encoder,
        thumbnail.as_raw(),
        thumbnail.width(),
        thumbnail.height(),
        image::ExtendedColorType::Rgba8,
    )
    .context("Failed to encode PNG")?;

    Ok(base64::engine::general_purpose::STANDARD.encode(&png_buf))
}

// -- File extraction helpers ------------------------------------------------

fn extract_file_from_bsa(archive_path: &Path, internal_path: &str) -> Result<Vec<u8>> {
    use ba2::{guess_format, FileFormat};

    let mut file = std::fs::File::open(archive_path)?;
    let format = guess_format(&mut file);

    match format {
        Some(FileFormat::TES3) => extract_from_bsa_tes3(archive_path, internal_path),
        Some(FileFormat::TES4) => extract_from_bsa_tes4(archive_path, internal_path),
        Some(FileFormat::FO4) => extract_file_from_ba2(archive_path, internal_path),
        None => bail!("Unrecognized BSA format"),
    }
}

fn extract_from_bsa_tes3(archive_path: &Path, internal_path: &str) -> Result<Vec<u8>> {
    use ba2::prelude::*;
    use ba2::tes3::{Archive, ArchiveKey};

    let archive: Archive<'static> = Archive::read(archive_path)
        .map_err(|e| anyhow::anyhow!("Failed to read TES3 BSA: {e}"))?;

    let normalized = normalize_archive_path(internal_path);

    // Try exact key lookup first
    let key: ArchiveKey = normalized.as_bytes().into();
    if let Some(file) = archive.get(&key) {
        let mut buf = Vec::new();
        file.write(&mut buf)?;
        return Ok(buf);
    }

    // Fallback: case-insensitive scan
    let lower = normalized.to_lowercase();
    for (k, file) in archive.iter() {
        if k.name().to_str_lossy().to_lowercase() == lower {
            let mut buf = Vec::new();
            file.write(&mut buf)?;
            return Ok(buf);
        }
    }

    bail!("File not found in TES3 BSA: {internal_path}")
}

fn extract_from_bsa_tes4(archive_path: &Path, internal_path: &str) -> Result<Vec<u8>> {
    use ba2::prelude::*;
    use ba2::tes4::Archive;

    let (archive, meta): (Archive<'static>, _) = Archive::read(archive_path)
        .map_err(|e| anyhow::anyhow!("Failed to read TES4 BSA: {e}"))?;

    let normalized = normalize_archive_path(internal_path);
    let lower = normalized.to_lowercase();
    let compression_opts: ba2::tes4::FileCompressionOptions = meta.into();

    for (dir_key, directory) in archive.iter() {
        let dir_name = dir_key.name().to_str_lossy().to_string();
        let dir_path = normalize_archive_path(&dir_name);

        for (file_key, file) in directory.iter() {
            let file_name = file_key.name().to_str_lossy().to_string();
            let full_path = if dir_path.is_empty() {
                normalize_archive_path(&file_name)
            } else {
                format!("{}/{}", dir_path, normalize_archive_path(&file_name))
            };

            if full_path.to_lowercase() == lower {
                let mut buf = Vec::new();
                file.write(&mut buf, &compression_opts)?;
                return Ok(buf);
            }
        }
    }

    bail!("File not found in TES4 BSA: {internal_path}")
}

fn extract_file_from_ba2(archive_path: &Path, internal_path: &str) -> Result<Vec<u8>> {
    use ba2::fo4::Archive;
    use ba2::prelude::*;

    let (archive, meta): (Archive<'static>, _) = Archive::read(archive_path)
        .map_err(|e| anyhow::anyhow!("Failed to read BA2: {e}"))?;

    let normalized = normalize_archive_path(internal_path);
    let lower = normalized.to_lowercase();
    let write_opts: ba2::fo4::FileWriteOptions = meta.into();

    // Case-insensitive search through entries
    for (key, file) in archive.iter() {
        let entry_path = key.name().to_str_lossy().to_string();
        if normalize_archive_path(&entry_path).to_lowercase() == lower {
            let mut buf = Vec::new();
            file.write(&mut buf, &write_opts)?;
            return Ok(buf);
        }
    }

    bail!("File not found in BA2: {internal_path}")
}

fn extract_file_from_zip(archive_path: &Path, internal_path: &str) -> Result<Vec<u8>> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    let normalized = normalize_archive_path(internal_path);

    // Try exact match first
    if let Ok(mut entry) = archive.by_name(&normalized) {
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf)?;
        return Ok(buf);
    }

    // Fallback: case-insensitive scan
    let lower = normalized.to_lowercase();
    for i in 0..archive.len() {
        let name = archive.by_index_raw(i)?.name().to_string();
        if normalize_archive_path(&name).to_lowercase() == lower {
            let mut entry = archive.by_index(i)?;
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buf)?;
            return Ok(buf);
        }
    }

    bail!("File not found in ZIP: {internal_path}")
}

fn extract_file_from_7z(archive_path: &Path, internal_path: &str) -> Result<Vec<u8>> {
    let normalized = normalize_archive_path(internal_path);
    let lower = normalized.to_lowercase();
    let mut result: Option<Vec<u8>> = None;

    let file = std::fs::File::open(archive_path)
        .with_context(|| format!("Failed to open 7z: {}", archive_path.display()))?;

    sevenz_rust2::decompress_with_extract_fn(
        file,
        ".",
        |entry, reader: &mut dyn Read, _dest| {
            if result.is_some() {
                return Ok(true);
            }
            let entry_path = normalize_archive_path(&entry.name().to_string());
            if entry_path.to_lowercase() == lower {
                let mut buf = Vec::new();
                reader.read_to_end(&mut buf)?;
                result = Some(buf);
            }
            Ok(true)
        },
    )
    .map_err(|e| anyhow::anyhow!("Failed to read 7z {}: {e}", archive_path.display()))?;

    result.ok_or_else(|| anyhow::anyhow!("File not found in 7z: {internal_path}"))
}

// ---------------------------------------------------------------------------
// Tree building
// ---------------------------------------------------------------------------

/// Convert a flat list of archive entries into a nested file tree for UI
/// rendering. Intermediate directories are synthesized if absent.
pub fn build_file_tree(entries: &[ArchiveEntry]) -> Vec<FileTreeNode> {
    // Use a trie-like approach: collect all paths, split by '/', insert into
    // a temporary HashMap-based tree, then convert to sorted FileTreeNode vec.
    let mut root_children: HashMap<String, TempNode> = HashMap::new();

    for entry in entries {
        let parts: Vec<&str> = entry.path.split('/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            continue;
        }
        insert_into_tree(&mut root_children, &parts, entry);
    }

    let mut nodes: Vec<FileTreeNode> = root_children
        .into_values()
        .map(|n| n.into_file_tree_node())
        .collect();
    sort_tree(&mut nodes);
    nodes
}

struct TempNode {
    name: String,
    path: String,
    is_dir: bool,
    file_size: u64,
    children: HashMap<String, TempNode>,
}

impl TempNode {
    fn into_file_tree_node(self) -> FileTreeNode {
        let mut children: Vec<FileTreeNode> = self
            .children
            .into_values()
            .map(|n| n.into_file_tree_node())
            .collect();
        sort_tree(&mut children);

        FileTreeNode {
            name: self.name,
            path: self.path,
            is_dir: self.is_dir,
            children,
            file_size: self.file_size,
        }
    }
}

fn insert_into_tree(
    children: &mut HashMap<String, TempNode>,
    parts: &[&str],
    entry: &ArchiveEntry,
) {
    if parts.is_empty() {
        return;
    }

    let key = parts[0].to_string();
    let is_leaf = parts.len() == 1;

    let node = children.entry(key.clone()).or_insert_with(|| {
        if is_leaf {
            TempNode {
                name: key.clone(),
                path: entry.path.clone(),
                is_dir: entry.is_dir,
                file_size: entry.file_size,
                children: HashMap::new(),
            }
        } else {
            // Intermediate directory — synthesize
            let dir_path = parts[..1].join("/");
            TempNode {
                name: key.clone(),
                path: dir_path,
                is_dir: true,
                file_size: 0,
                children: HashMap::new(),
            }
        }
    });

    if !is_leaf {
        // Update the intermediate node's path to be the correct prefix
        let prefix: String = parts.iter().take(1).copied().collect::<Vec<_>>().join("/");
        if node.path.len() < prefix.len() {
            node.path = prefix;
        }
        node.is_dir = true;
        insert_into_tree(&mut node.children, &parts[1..], entry);
    }
}

/// Sort tree nodes: directories first, then alphabetically by name.
fn sort_tree(nodes: &mut [FileTreeNode]) {
    nodes.sort_by(|a, b| {
        // Directories before files
        b.is_dir.cmp(&a.is_dir).then_with(|| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
        })
    });
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Normalize an archive-internal path: backslashes to forward slashes,
/// strip leading `./` or `/`, collapse repeated slashes, trim trailing slash.
fn normalize_archive_path(raw: &str) -> String {
    let replaced = raw.replace('\\', "/");
    let trimmed = replaced
        .trim_start_matches('/')
        .trim_start_matches("./")
        .trim_end_matches('/');
    // Collapse double slashes
    let mut result = String::with_capacity(trimmed.len());
    let mut prev_slash = false;
    for ch in trimmed.chars() {
        if ch == '/' {
            if !prev_slash {
                result.push(ch);
            }
            prev_slash = true;
        } else {
            result.push(ch);
            prev_slash = false;
        }
    }
    result
}

/// Extract the leaf file/dir name from a normalized path.
fn leaf_name(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Path helpers -------------------------------------------------------

    #[test]
    fn test_normalize_archive_path_backslashes() {
        assert_eq!(
            normalize_archive_path(r"textures\sky\clouds.dds"),
            "textures/sky/clouds.dds"
        );
    }

    #[test]
    fn test_normalize_archive_path_leading_slash() {
        assert_eq!(
            normalize_archive_path("/textures/sky/clouds.dds"),
            "textures/sky/clouds.dds"
        );
    }

    #[test]
    fn test_normalize_archive_path_dot_slash() {
        assert_eq!(
            normalize_archive_path("./meshes/actor.nif"),
            "meshes/actor.nif"
        );
    }

    #[test]
    fn test_normalize_archive_path_double_slash() {
        assert_eq!(
            normalize_archive_path("textures//sky//clouds.dds"),
            "textures/sky/clouds.dds"
        );
    }

    #[test]
    fn test_normalize_archive_path_trailing_slash() {
        assert_eq!(
            normalize_archive_path("textures/sky/"),
            "textures/sky"
        );
    }

    #[test]
    fn test_leaf_name() {
        assert_eq!(leaf_name("textures/sky/clouds.dds"), "clouds.dds");
        assert_eq!(leaf_name("clouds.dds"), "clouds.dds");
        assert_eq!(leaf_name(""), "");
    }

    // -- Tree building ------------------------------------------------------

    #[test]
    fn test_build_file_tree_empty() {
        let tree = build_file_tree(&[]);
        assert!(tree.is_empty());
    }

    #[test]
    fn test_build_file_tree_flat_files() {
        let entries = vec![
            ArchiveEntry {
                name: "readme.txt".into(),
                path: "readme.txt".into(),
                is_dir: false,
                file_size: 100,
                compressed_size: 50,
            },
            ArchiveEntry {
                name: "license.txt".into(),
                path: "license.txt".into(),
                is_dir: false,
                file_size: 200,
                compressed_size: 0,
            },
        ];

        let tree = build_file_tree(&entries);
        assert_eq!(tree.len(), 2);
        // Sorted alphabetically: license.txt before readme.txt
        assert_eq!(tree[0].name, "license.txt");
        assert_eq!(tree[1].name, "readme.txt");
        assert!(!tree[0].is_dir);
        assert!(tree[0].children.is_empty());
    }

    #[test]
    fn test_build_file_tree_nested() {
        let entries = vec![
            ArchiveEntry {
                name: "clouds.dds".into(),
                path: "textures/sky/clouds.dds".into(),
                is_dir: false,
                file_size: 1024,
                compressed_size: 512,
            },
            ArchiveEntry {
                name: "sun.dds".into(),
                path: "textures/sky/sun.dds".into(),
                is_dir: false,
                file_size: 2048,
                compressed_size: 0,
            },
            ArchiveEntry {
                name: "body.nif".into(),
                path: "meshes/actors/body.nif".into(),
                is_dir: false,
                file_size: 4096,
                compressed_size: 0,
            },
        ];

        let tree = build_file_tree(&entries);
        // Root should have 2 dirs: meshes, textures
        assert_eq!(tree.len(), 2);
        assert!(tree[0].is_dir);
        assert!(tree[1].is_dir);
        // Dirs sorted alphabetically
        assert_eq!(tree[0].name, "meshes");
        assert_eq!(tree[1].name, "textures");

        // meshes -> actors -> body.nif
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].name, "actors");
        assert!(tree[0].children[0].is_dir);
        assert_eq!(tree[0].children[0].children.len(), 1);
        assert_eq!(tree[0].children[0].children[0].name, "body.nif");
        assert!(!tree[0].children[0].children[0].is_dir);
        assert_eq!(tree[0].children[0].children[0].file_size, 4096);

        // textures -> sky -> {clouds.dds, sun.dds}
        assert_eq!(tree[1].children.len(), 1);
        let sky = &tree[1].children[0];
        assert_eq!(sky.name, "sky");
        assert_eq!(sky.children.len(), 2);
        assert_eq!(sky.children[0].name, "clouds.dds");
        assert_eq!(sky.children[1].name, "sun.dds");
    }

    #[test]
    fn test_build_file_tree_dirs_before_files() {
        let entries = vec![
            ArchiveEntry {
                name: "readme.txt".into(),
                path: "readme.txt".into(),
                is_dir: false,
                file_size: 100,
                compressed_size: 0,
            },
            ArchiveEntry {
                name: "clouds.dds".into(),
                path: "textures/clouds.dds".into(),
                is_dir: false,
                file_size: 1024,
                compressed_size: 0,
            },
        ];

        let tree = build_file_tree(&entries);
        assert_eq!(tree.len(), 2);
        // Directory "textures" should come before file "readme.txt"
        assert!(tree[0].is_dir);
        assert_eq!(tree[0].name, "textures");
        assert!(!tree[1].is_dir);
        assert_eq!(tree[1].name, "readme.txt");
    }

    #[test]
    fn test_build_file_tree_with_explicit_directory_entries() {
        let entries = vec![
            ArchiveEntry {
                name: "textures".into(),
                path: "textures".into(),
                is_dir: true,
                file_size: 0,
                compressed_size: 0,
            },
            ArchiveEntry {
                name: "clouds.dds".into(),
                path: "textures/clouds.dds".into(),
                is_dir: false,
                file_size: 1024,
                compressed_size: 0,
            },
        ];

        let tree = build_file_tree(&entries);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "textures");
        assert!(tree[0].is_dir);
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].name, "clouds.dds");
    }

    #[test]
    fn test_build_file_tree_deeply_nested() {
        let entries = vec![ArchiveEntry {
            name: "deep.txt".into(),
            path: "a/b/c/d/e/deep.txt".into(),
            is_dir: false,
            file_size: 42,
            compressed_size: 0,
        }];

        let tree = build_file_tree(&entries);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "a");

        let mut node = &tree[0];
        for expected in &["b", "c", "d", "e"] {
            assert_eq!(node.children.len(), 1);
            node = &node.children[0];
            assert_eq!(node.name, *expected);
            assert!(node.is_dir);
        }
        assert_eq!(node.children.len(), 1);
        assert_eq!(node.children[0].name, "deep.txt");
        assert!(!node.children[0].is_dir);
    }

    // -- ArchiveContents construction (unit-level) --------------------------

    #[test]
    fn test_archive_contents_serializable() {
        let contents = ArchiveContents {
            archive_type: "zip".into(),
            total_files: 1,
            total_size: 100,
            entries: vec![ArchiveEntry {
                name: "test.txt".into(),
                path: "test.txt".into(),
                is_dir: false,
                file_size: 100,
                compressed_size: 50,
            }],
        };

        let json = serde_json::to_string(&contents).expect("should serialize");
        assert!(json.contains("\"archive_type\":\"zip\""));
        assert!(json.contains("\"total_files\":1"));
    }

    #[test]
    fn test_file_tree_node_serializable() {
        let node = FileTreeNode {
            name: "test".into(),
            path: "test".into(),
            is_dir: true,
            children: vec![FileTreeNode {
                name: "file.txt".into(),
                path: "test/file.txt".into(),
                is_dir: false,
                children: vec![],
                file_size: 42,
            }],
            file_size: 0,
        };

        let json = serde_json::to_string(&node).expect("should serialize");
        assert!(json.contains("\"name\":\"test\""));
        assert!(json.contains("\"children\":["));
    }
}
