//! Batch DDS texture downscaling for mod staging directories.
//!
//! Scans a mod's staging folder for `.dds` textures, identifies those exceeding
//! a target resolution, and downscales them using Lanczos3 while preserving the
//! original BC compression format. Normal maps and mask textures can be skipped
//! to avoid visual artifacts from aggressive downscaling.
//!
//! Uses rayon for parallel processing and the existing `image_dds` / `ddsfile`
//! crates that the rest of the codebase already depends on.

use ddsfile::Dds;
use image::imageops::FilterType;
use image_dds::{dds_from_image, image_from_dds, ImageFormat};
use log::{debug, info, warn};
use rayon::prelude::*;
use serde::Serialize;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for batch texture optimization.
#[derive(Debug, Clone)]
pub struct TextureOptConfig {
    /// Maximum resolution on either axis. Textures with both dimensions at or
    /// below this value are left untouched.
    pub max_resolution: u32,
    /// Skip normal maps (filenames containing `_n.`, `_normal.`, `_msn.`, etc.).
    pub skip_normals: bool,
    /// Skip mask/specular maps (`_m.`, `_s.`, `_g.`, `_p.`, `_em.`, `_sk.`).
    pub skip_masks: bool,
    /// When true, scan and report what *would* change without writing anything.
    pub dry_run: bool,
}

impl Default for TextureOptConfig {
    fn default() -> Self {
        Self {
            max_resolution: 2048,
            skip_normals: true,
            skip_masks: false,
            dry_run: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

/// Aggregate result of a batch optimization run.
#[derive(Debug, Clone, Serialize)]
pub struct TextureOptResult {
    /// Total DDS files found.
    pub files_scanned: usize,
    /// Files that were actually rewritten (or would be, in dry_run mode).
    pub files_processed: usize,
    /// Files skipped (already below target, normal maps, errors, etc.).
    pub files_skipped: usize,
    /// Sum of original file sizes for processed textures.
    pub original_bytes: u64,
    /// Sum of new file sizes for processed textures.
    pub new_bytes: u64,
    /// Per-file detail.
    pub file_results: Vec<TextureFileResult>,
    /// Errors encountered (non-fatal — one bad file doesn't abort the batch).
    pub errors: Vec<String>,
}

impl TextureOptResult {
    /// Total bytes saved (positive = smaller).
    pub fn bytes_saved(&self) -> i64 {
        self.original_bytes as i64 - self.new_bytes as i64
    }
}

/// Per-file result of a texture optimization.
#[derive(Debug, Clone, Serialize)]
pub struct TextureFileResult {
    /// Path relative to the staging directory root.
    pub relative_path: PathBuf,
    pub original_width: u32,
    pub original_height: u32,
    /// `None` when skipped.
    pub new_width: Option<u32>,
    /// `None` when skipped.
    pub new_height: Option<u32>,
    pub original_size: u64,
    /// `None` when skipped or dry_run.
    pub new_size: Option<u64>,
    /// If the file was skipped, explains why.
    pub skipped_reason: Option<String>,
}

// ---------------------------------------------------------------------------
// Normal / mask detection
// ---------------------------------------------------------------------------

/// Filename suffixes (before the `.dds` extension) that indicate a normal map.
const NORMAL_MAP_SUFFIXES: &[&str] = &[
    "_n", "_normal", "_nrm", "_msn", "_normals",
];

/// Filename suffixes that indicate a mask, specular, glow, or similar map that
/// is best left at full resolution.
const MASK_SUFFIXES: &[&str] = &[
    "_m", "_s", "_g", "_p", "_em", "_sk", "_spec", "_mask", "_glow",
    "_emissive", "_specular",
];

/// Returns `true` if the filename (case-insensitive) looks like a normal map.
pub fn is_normal_map(path: &Path) -> bool {
    has_texture_suffix(path, NORMAL_MAP_SUFFIXES)
}

/// Returns `true` if the filename looks like a mask / specular / glow map.
pub fn is_mask_texture(path: &Path) -> bool {
    has_texture_suffix(path, MASK_SUFFIXES)
}

fn has_texture_suffix(path: &Path, suffixes: &[&str]) -> bool {
    let stem = match path.file_stem().and_then(|s| s.to_str()) {
        Some(s) => s.to_lowercase(),
        None => return false,
    };
    suffixes.iter().any(|suffix| stem.ends_with(suffix))
}

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------

/// Scan a mod's staging directory and report the resolution of every `.dds`
/// file found. Does **not** modify any files.
pub fn scan_mod_textures(staging_path: &Path) -> Result<Vec<TextureFileResult>, String> {
    let dds_paths = collect_dds_files(staging_path);
    let mut results = Vec::with_capacity(dds_paths.len());

    for (rel, abs) in &dds_paths {
        match read_dds_dimensions(abs) {
            Ok((w, h, size)) => {
                results.push(TextureFileResult {
                    relative_path: rel.clone(),
                    original_width: w,
                    original_height: h,
                    new_width: None,
                    new_height: None,
                    original_size: size,
                    new_size: None,
                    skipped_reason: None,
                });
            }
            Err(e) => {
                results.push(TextureFileResult {
                    relative_path: rel.clone(),
                    original_width: 0,
                    original_height: 0,
                    new_width: None,
                    new_height: None,
                    original_size: 0,
                    new_size: None,
                    skipped_reason: Some(format!("Read error: {}", e)),
                });
            }
        }
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Optimization
// ---------------------------------------------------------------------------

/// Downscale all DDS textures in `staging_path` that exceed `config.max_resolution`.
///
/// The `progress` callback receives `(completed, total)` after each file.
pub fn optimize_mod_textures(
    staging_path: &Path,
    config: &TextureOptConfig,
    progress: impl Fn(usize, usize) + Send + Sync,
) -> TextureOptResult {
    let dds_paths = collect_dds_files(staging_path);
    let total = dds_paths.len();

    info!(
        "Texture optimizer: {} DDS files in {}, max_res={}, dry_run={}",
        total,
        staging_path.display(),
        config.max_resolution,
        config.dry_run
    );

    let completed = std::sync::atomic::AtomicUsize::new(0);

    let per_file_results: Vec<Result<TextureFileResult, String>> = dds_paths
        .par_iter()
        .map(|(rel, abs)| {
            let result = process_single_texture(rel, abs, config);
            let done = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            progress(done, total);
            result
        })
        .collect();

    // Aggregate
    let mut result = TextureOptResult {
        files_scanned: total,
        files_processed: 0,
        files_skipped: 0,
        original_bytes: 0,
        new_bytes: 0,
        file_results: Vec::with_capacity(total),
        errors: Vec::new(),
    };

    for item in per_file_results {
        match item {
            Ok(fr) => {
                if fr.skipped_reason.is_some() {
                    result.files_skipped += 1;
                } else {
                    result.files_processed += 1;
                    result.original_bytes += fr.original_size;
                    result.new_bytes += fr.new_size.unwrap_or(fr.original_size);
                }
                result.file_results.push(fr);
            }
            Err(e) => {
                result.files_skipped += 1;
                result.errors.push(e);
            }
        }
    }

    info!(
        "Texture optimizer complete: {}/{} processed, {} skipped, saved {} bytes",
        result.files_processed,
        result.files_scanned,
        result.files_skipped,
        result.bytes_saved()
    );

    result
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Collect all `.dds` files under `root`, returning `(relative_path, absolute_path)`.
fn collect_dds_files(root: &Path) -> Vec<(PathBuf, PathBuf)> {
    WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .map(|ext| ext.eq_ignore_ascii_case("dds"))
                    .unwrap_or(false)
        })
        .map(|e| {
            let abs = e.path().to_path_buf();
            let rel = abs.strip_prefix(root).unwrap_or(&abs).to_path_buf();
            (rel, abs)
        })
        .collect()
}

/// Read just the dimensions and file size from a DDS without decoding pixels.
fn read_dds_dimensions(path: &Path) -> Result<(u32, u32, u64), String> {
    let data =
        std::fs::read(path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let file_size = data.len() as u64;
    let dds = Dds::read(&mut Cursor::new(&data))
        .map_err(|e| format!("Failed to parse DDS {}: {}", path.display(), e))?;
    Ok((dds.header.width, dds.header.height, file_size))
}

/// Detect the `ImageFormat` from a parsed DDS file.
///
/// Prefers the DX10 extended header (`header10.dxgi_format`) when present,
/// otherwise falls back to the legacy FourCC in the pixel format header.
fn detect_image_format(dds: &Dds) -> ImageFormat {
    // DX10 extended header takes priority
    if let Some(ref h10) = dds.header10 {
        return dxgi_u32_to_image_format(h10.dxgi_format as u32);
    }

    // Legacy pixel format FourCC
    let pf = &dds.header.spf;
    let fourcc_val = match pf.fourcc {
        Some(ref fcc) => fcc.0,
        None => 0u32,
    };
    let fourcc_bytes = fourcc_val.to_le_bytes();
    match &fourcc_bytes {
        b"DXT1" => ImageFormat::BC1RgbaUnorm,
        b"DXT3" => ImageFormat::BC2RgbaUnorm,
        b"DXT5" => ImageFormat::BC3RgbaUnorm,
        b"ATI1" | b"BC4U" => ImageFormat::BC4RUnorm,
        b"ATI2" | b"BC5U" => ImageFormat::BC5RgUnorm,
        _ => {
            // Uncompressed RGBA fallback
            if pf.rgb_bit_count.unwrap_or(0) == 32 {
                ImageFormat::Rgba8Unorm
            } else {
                warn!(
                    "Unknown DDS pixel format (fourcc {:?}), defaulting to BC7",
                    fourcc_bytes
                );
                ImageFormat::BC7RgbaUnorm
            }
        }
    }
}

/// Map DXGI_FORMAT u32 to image_dds ImageFormat (mirrors wabbajack_directives).
fn dxgi_u32_to_image_format(dxgi: u32) -> ImageFormat {
    match dxgi {
        28 => ImageFormat::Rgba8Unorm,
        71 => ImageFormat::BC1RgbaUnorm,
        72 => ImageFormat::BC1RgbaUnormSrgb,
        74 => ImageFormat::BC2RgbaUnorm,
        75 => ImageFormat::BC2RgbaUnormSrgb,
        77 => ImageFormat::BC3RgbaUnorm,
        78 => ImageFormat::BC3RgbaUnormSrgb,
        80 => ImageFormat::BC4RUnorm,
        81 => ImageFormat::BC4RSnorm,
        83 => ImageFormat::BC5RgUnorm,
        84 => ImageFormat::BC5RgSnorm,
        87 => ImageFormat::Bgra8Unorm,
        95 => ImageFormat::BC6hRgbUfloat,
        96 => ImageFormat::BC6hRgbSfloat,
        98 => ImageFormat::BC7RgbaUnorm,
        99 => ImageFormat::BC7RgbaUnormSrgb,
        other => {
            warn!("Unknown DXGI_FORMAT {}, defaulting to BC7_UNorm", other);
            ImageFormat::BC7RgbaUnorm
        }
    }
}

/// Calculate the new dimensions, halving until both axes are <= max_resolution.
/// Maintains aspect ratio by dividing both dimensions by the same power of 2.
fn target_dimensions(width: u32, height: u32, max_res: u32) -> (u32, u32) {
    let mut w = width;
    let mut h = height;
    while w > max_res || h > max_res {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    (w, h)
}

/// Process a single DDS texture: skip-check, decode, resize, re-encode.
fn process_single_texture(
    rel: &Path,
    abs: &Path,
    config: &TextureOptConfig,
) -> Result<TextureFileResult, String> {
    // Read file
    let data = std::fs::read(abs)
        .map_err(|e| format!("Failed to read {}: {}", abs.display(), e))?;
    let original_size = data.len() as u64;

    let dds = Dds::read(&mut Cursor::new(&data))
        .map_err(|e| format!("Failed to parse DDS {}: {}", abs.display(), e))?;

    let width = dds.header.width;
    let height = dds.header.height;

    // Check if below target already
    if width <= config.max_resolution && height <= config.max_resolution {
        return Ok(TextureFileResult {
            relative_path: rel.to_path_buf(),
            original_width: width,
            original_height: height,
            new_width: None,
            new_height: None,
            original_size,
            new_size: None,
            skipped_reason: Some(format!(
                "Already within target ({}x{} <= {})",
                width, height, config.max_resolution
            )),
        });
    }

    // Check normal map skip
    if config.skip_normals && is_normal_map(abs) {
        return Ok(TextureFileResult {
            relative_path: rel.to_path_buf(),
            original_width: width,
            original_height: height,
            new_width: None,
            new_height: None,
            original_size,
            new_size: None,
            skipped_reason: Some("Normal map (skipped by config)".to_string()),
        });
    }

    // Check mask skip
    if config.skip_masks && is_mask_texture(abs) {
        return Ok(TextureFileResult {
            relative_path: rel.to_path_buf(),
            original_width: width,
            original_height: height,
            new_width: None,
            new_height: None,
            original_size,
            new_size: None,
            skipped_reason: Some("Mask/specular texture (skipped by config)".to_string()),
        });
    }

    let (new_w, new_h) = target_dimensions(width, height, config.max_resolution);
    let format = detect_image_format(&dds);

    // Dry run — report what would happen without writing
    if config.dry_run {
        return Ok(TextureFileResult {
            relative_path: rel.to_path_buf(),
            original_width: width,
            original_height: height,
            new_width: Some(new_w),
            new_height: Some(new_h),
            original_size,
            new_size: None,
            skipped_reason: None,
        });
    }

    // Decode DDS to RGBA (mip level 0)
    let rgba = image_from_dds(&dds, 0)
        .map_err(|e| format!("Failed to decode DDS {}: {}", abs.display(), e))?;

    // Resize with Lanczos3
    let resized = image::imageops::resize(&rgba, new_w, new_h, FilterType::Lanczos3);

    // Re-encode preserving original format, with auto mipmaps
    let new_dds = dds_from_image(
        &resized,
        format,
        image_dds::Quality::Normal,
        image_dds::Mipmaps::GeneratedAutomatic,
    )
    .map_err(|e| format!("Failed to encode DDS {} ({:?}): {}", abs.display(), format, e))?;

    // Write back to same path
    let mut file = std::fs::File::create(abs)
        .map_err(|e| format!("Failed to create {}: {}", abs.display(), e))?;
    new_dds
        .write(&mut file)
        .map_err(|e| format!("Failed to write DDS {}: {}", abs.display(), e))?;

    let new_size = std::fs::metadata(abs)
        .map(|m| m.len())
        .unwrap_or(0);

    debug!(
        "Optimized texture: {} {}x{} -> {}x{} ({} -> {} bytes, {:?})",
        rel.display(),
        width,
        height,
        new_w,
        new_h,
        original_size,
        new_size,
        format
    );

    Ok(TextureFileResult {
        relative_path: rel.to_path_buf(),
        original_width: width,
        original_height: height,
        new_width: Some(new_w),
        new_height: Some(new_h),
        original_size,
        new_size: Some(new_size),
        skipped_reason: None,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_target_dimensions_halves_correctly() {
        assert_eq!(target_dimensions(4096, 4096, 2048), (2048, 2048));
        assert_eq!(target_dimensions(4096, 4096, 1024), (1024, 1024));
        assert_eq!(target_dimensions(4096, 2048, 2048), (2048, 1024));
        assert_eq!(target_dimensions(8192, 4096, 2048), (2048, 1024));
        // Already within target
        assert_eq!(target_dimensions(1024, 512, 2048), (1024, 512));
        // Minimum of 1
        assert_eq!(target_dimensions(2, 2, 1), (1, 1));
    }

    #[test]
    fn test_is_normal_map_detection() {
        assert!(is_normal_map(Path::new("textures/rock_n.dds")));
        assert!(is_normal_map(Path::new("textures/rock_normal.dds")));
        assert!(is_normal_map(Path::new("textures/ROCK_N.DDS")));
        assert!(is_normal_map(Path::new("textures/stone_msn.dds")));
        assert!(is_normal_map(Path::new("textures/wall_nrm.dds")));
        // Should NOT match
        assert!(!is_normal_map(Path::new("textures/rock_d.dds")));
        assert!(!is_normal_map(Path::new("textures/rock.dds")));
        assert!(!is_normal_map(Path::new("textures/innocent.dds")));
    }

    #[test]
    fn test_is_mask_texture_detection() {
        assert!(is_mask_texture(Path::new("textures/rock_m.dds")));
        assert!(is_mask_texture(Path::new("textures/rock_s.dds")));
        assert!(is_mask_texture(Path::new("textures/rock_spec.dds")));
        assert!(is_mask_texture(Path::new("textures/rock_emissive.dds")));
        assert!(is_mask_texture(Path::new("textures/ROCK_GLOW.DDS")));
        // Should NOT match
        assert!(!is_mask_texture(Path::new("textures/rock_d.dds")));
        assert!(!is_mask_texture(Path::new("textures/rock_n.dds")));
    }

    #[test]
    fn test_collect_dds_files_finds_dds_only() {
        let dir = TempDir::new().unwrap();
        let textures = dir.path().join("textures");
        fs::create_dir_all(&textures).unwrap();

        fs::write(textures.join("diffuse.dds"), b"fake dds").unwrap();
        fs::write(textures.join("normal.DDS"), b"fake dds").unwrap();
        fs::write(textures.join("readme.txt"), b"not a texture").unwrap();
        fs::write(dir.path().join("root.dds"), b"fake dds").unwrap();

        let files = collect_dds_files(dir.path());
        assert_eq!(files.len(), 3, "Should find 3 .dds files, got {:?}", files);

        // All relative paths should not be absolute
        for (rel, _abs) in &files {
            assert!(
                !rel.is_absolute(),
                "Relative path should not be absolute: {}",
                rel.display()
            );
        }
    }

    #[test]
    fn test_scan_empty_directory() {
        let dir = TempDir::new().unwrap();
        let results = scan_mod_textures(dir.path()).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_optimize_empty_directory() {
        let dir = TempDir::new().unwrap();
        let config = TextureOptConfig::default();
        let result = optimize_mod_textures(dir.path(), &config, |_, _| {});
        assert_eq!(result.files_scanned, 0);
        assert_eq!(result.files_processed, 0);
        assert_eq!(result.files_skipped, 0);
        assert_eq!(result.bytes_saved(), 0);
    }

    #[test]
    fn test_default_config() {
        let config = TextureOptConfig::default();
        assert_eq!(config.max_resolution, 2048);
        assert!(config.skip_normals);
        assert!(!config.skip_masks);
        assert!(!config.dry_run);
    }

    #[test]
    fn test_dxgi_format_mapping_common_values() {
        assert_eq!(dxgi_u32_to_image_format(71), ImageFormat::BC1RgbaUnorm);
        assert_eq!(dxgi_u32_to_image_format(77), ImageFormat::BC3RgbaUnorm);
        assert_eq!(dxgi_u32_to_image_format(80), ImageFormat::BC4RUnorm);
        assert_eq!(dxgi_u32_to_image_format(83), ImageFormat::BC5RgUnorm);
        assert_eq!(dxgi_u32_to_image_format(98), ImageFormat::BC7RgbaUnorm);
        assert_eq!(dxgi_u32_to_image_format(95), ImageFormat::BC6hRgbUfloat);
        assert_eq!(dxgi_u32_to_image_format(28), ImageFormat::Rgba8Unorm);
        // Unknown should default to BC7
        assert_eq!(dxgi_u32_to_image_format(9999), ImageFormat::BC7RgbaUnorm);
    }

    #[test]
    fn test_bytes_saved_calculation() {
        let result = TextureOptResult {
            files_scanned: 2,
            files_processed: 2,
            files_skipped: 0,
            original_bytes: 10_000,
            new_bytes: 3_000,
            file_results: vec![],
            errors: vec![],
        };
        assert_eq!(result.bytes_saved(), 7_000);
    }
}
