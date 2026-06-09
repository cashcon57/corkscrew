//! GPU-accelerated texture encoding for DDS files.
//!
//! BC7/BC6H encoding uses parallel CPU encoding via image_dds with
//! budget-aware batch sizing. GPU detection uses platform-native APIs
//! (system_profiler on macOS, lspci/sysfs on Linux).

use image_dds::{dds_from_image, ImageFormat};
use log::{debug, info};
use rayon::prelude::*;
use serde::Serialize;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// GPU info and VRAM budget
// ---------------------------------------------------------------------------

/// GPU device information.
#[derive(Debug, Clone, Serialize)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: String,
    pub device_type: String,
    pub backend: String,
    pub vram_bytes: u64,
    pub budget_bytes: u64,
}

/// Detect available GPU using platform-native APIs.
pub fn detect_gpu() -> Option<GpuInfo> {
    #[cfg(target_os = "macos")]
    {
        detect_gpu_macos()
    }

    #[cfg(target_os = "linux")]
    {
        detect_gpu_linux()
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
fn detect_gpu_macos() -> Option<GpuInfo> {
    let output = std::process::Command::new("system_profiler")
        .arg("SPDisplaysDataType")
        .arg("-json")
        .output()
        .ok()?;

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let displays = json.get("SPDisplaysDataType")?.as_array()?;
    let first = displays.first()?;

    let name = first
        .get("sppci_model")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown GPU")
        .to_string();

    let vram_str = first
        .get("spdisplays_vram")
        .or_else(|| first.get("spdisplays_vram_shared"))
        .and_then(|v| v.as_str())
        .unwrap_or("0");

    // Parse VRAM like "8 GB" or "1536 MB"
    let vram_bytes = parse_vram_string(vram_str);
    let is_integrated = name.contains("Apple") || name.contains("Intel");
    let budget_pct = if is_integrated { 0.12 } else { 0.25 };
    let budget_bytes = (vram_bytes as f64 * budget_pct) as u64;

    info!(
        "GPU detected: {} (~{}MB VRAM, {}MB budget, Metal)",
        name,
        vram_bytes / (1024 * 1024),
        budget_bytes / (1024 * 1024),
    );

    Some(GpuInfo {
        name,
        vendor: "Apple".to_string(),
        device_type: if is_integrated {
            "integrated"
        } else {
            "discrete"
        }
        .to_string(),
        backend: "Metal".to_string(),
        vram_bytes,
        budget_bytes,
    })
}

#[cfg(target_os = "linux")]
fn detect_gpu_linux() -> Option<GpuInfo> {
    // Try lspci first
    let output = std::process::Command::new("lspci").output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    for line in stdout.lines() {
        if line.contains("VGA")
            || line.contains("3D controller")
            || line.contains("Display controller")
        {
            let name = line.split(':').last()?.trim().to_string();
            if name.is_empty() {
                continue;
            }

            let is_integrated = name.contains("Intel")
                || (name.contains("AMD/ATI")
                    && (name.contains("Renoir")
                        || name.contains("Cezanne")
                        || name.contains("Barcelo")
                        || name.contains("Van Gogh") // Steam Deck
                        || name.contains("Sephiroth") // Steam Deck OLED
                        || name.contains("Rembrandt")));

            let vram_bytes = if is_integrated {
                1024 * 1024 * 1024 // 1GB estimate for integrated
            } else {
                4 * 1024 * 1024 * 1024u64 // 4GB estimate for discrete
            };

            let budget_pct = if is_integrated { 0.12 } else { 0.25 };
            let budget_bytes = (vram_bytes as f64 * budget_pct) as u64;

            let vendor = if name.contains("NVIDIA") || name.contains("GeForce") {
                "NVIDIA"
            } else if name.contains("AMD") || name.contains("Radeon") {
                "AMD"
            } else if name.contains("Intel") {
                "Intel"
            } else {
                "Unknown"
            };

            info!(
                "GPU detected: {} (~{}MB VRAM, {}MB budget, Vulkan)",
                name,
                vram_bytes / (1024 * 1024),
                budget_bytes / (1024 * 1024),
            );

            return Some(GpuInfo {
                name,
                vendor: vendor.to_string(),
                device_type: if is_integrated {
                    "integrated"
                } else {
                    "discrete"
                }
                .to_string(),
                backend: "Vulkan".to_string(),
                vram_bytes,
                budget_bytes,
            });
        }
    }

    None
}

fn parse_vram_string(s: &str) -> u64 {
    let lower = s.to_lowercase();
    let num: f64 = lower
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .find(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);

    if lower.contains("gb") {
        (num * 1024.0 * 1024.0 * 1024.0) as u64
    } else if lower.contains("mb") {
        (num * 1024.0 * 1024.0) as u64
    } else {
        (num * 1024.0 * 1024.0) as u64 // Assume MB
    }
}

// ---------------------------------------------------------------------------
// Texture encoding batch
// ---------------------------------------------------------------------------

/// A batch of textures to encode with budget-aware sizing.
pub struct EncodeBatch {
    entries: Vec<EncodeEntry>,
    budget_bytes: u64,
    current_bytes: u64,
}

struct EncodeEntry {
    source_path: PathBuf,
    dest_path: PathBuf,
    format: DdsFormat,
    #[allow(dead_code)]
    width: u32,
    #[allow(dead_code)]
    height: u32,
    generate_mipmaps: bool,
}

/// Target DDS format for encoding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DdsFormat {
    BC1,
    BC3,
    BC4,
    BC5,
    BC7,
    BC6H,
    Rgba8,
}

impl EncodeBatch {
    pub fn new(budget_bytes: u64) -> Self {
        Self {
            entries: Vec::new(),
            budget_bytes,
            current_bytes: 0,
        }
    }

    /// Add a texture to the batch.
    pub fn add(
        &mut self,
        source: PathBuf,
        dest: PathBuf,
        format: DdsFormat,
        width: u32,
        height: u32,
        generate_mipmaps: bool,
    ) {
        let estimated_size = (width as u64) * (height as u64) * 4; // RGBA8
        self.current_bytes += estimated_size;
        self.entries.push(EncodeEntry {
            source_path: source,
            dest_path: dest,
            format,
            width,
            height,
            generate_mipmaps,
        });
    }

    /// Check if the batch should be flushed (budget exceeded).
    pub fn should_flush(&self) -> bool {
        self.current_bytes >= self.budget_bytes
    }

    /// Number of pending textures.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Process all entries in the batch using parallel CPU encoding.
    ///
    /// Returns (successes, failures).
    pub fn flush(&mut self) -> (usize, Vec<String>) {
        let entries: Vec<_> = self.entries.drain(..).collect();
        self.current_bytes = 0;

        let results: Vec<Result<(), String>> = entries
            .par_iter()
            .map(|entry| encode_texture_cpu(entry))
            .collect();

        let mut successes = 0;
        let mut failures = Vec::new();

        for result in results {
            match result {
                Ok(()) => successes += 1,
                Err(e) => failures.push(e),
            }
        }

        (successes, failures)
    }
}

/// Map our DdsFormat enum to image_dds ImageFormat.
fn dds_format_to_image_format(format: DdsFormat) -> ImageFormat {
    match format {
        DdsFormat::BC1 => ImageFormat::BC1RgbaUnorm,
        DdsFormat::BC3 => ImageFormat::BC3RgbaUnorm,
        DdsFormat::BC4 => ImageFormat::BC4RUnorm,
        DdsFormat::BC5 => ImageFormat::BC5RgUnorm,
        DdsFormat::BC7 => ImageFormat::BC7RgbaUnorm,
        DdsFormat::BC6H => ImageFormat::BC6hRgbUfloat,
        DdsFormat::Rgba8 => ImageFormat::Rgba8Unorm,
    }
}

/// Encode a single texture using CPU (image_dds).
fn encode_texture_cpu(entry: &EncodeEntry) -> Result<(), String> {
    let img = image::ImageReader::open(&entry.source_path)
        .map_err(|e| format!("Failed to open {}: {}", entry.source_path.display(), e))?
        .decode()
        .map_err(|e| format!("Failed to decode {}: {}", entry.source_path.display(), e))?;

    let rgba = img.to_rgba8();

    let target_format = dds_format_to_image_format(entry.format);

    let quality = image_dds::Quality::Normal;
    let mipmaps = if entry.generate_mipmaps {
        image_dds::Mipmaps::GeneratedAutomatic
    } else {
        image_dds::Mipmaps::Disabled
    };

    let dds = dds_from_image(&rgba, target_format, quality, mipmaps)
        .map_err(|e| format!("Failed to encode {}: {}", entry.source_path.display(), e))?;

    if let Some(parent) = entry.dest_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir: {}", e))?;
    }

    let mut file = std::fs::File::create(&entry.dest_path)
        .map_err(|e| format!("Failed to create {}: {}", entry.dest_path.display(), e))?;

    dds.write(&mut file)
        .map_err(|e| format!("Failed to write DDS {}: {}", entry.dest_path.display(), e))?;

    debug!(
        "Encoded texture: {} -> {} ({:?})",
        entry.source_path.display(),
        entry.dest_path.display(),
        entry.format
    );

    Ok(())
}

/// Get the GPU name for INI fixes (exposed for display_fix.rs).
pub fn get_gpu_name() -> Option<String> {
    detect_gpu().map(|info| info.name)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_gpu() {
        // Should not crash, may return None in CI
        let _gpu = detect_gpu();
    }

    #[test]
    fn test_encode_batch_budget() {
        let mut batch = EncodeBatch::new(1024 * 1024); // 1MB budget
        assert!(!batch.should_flush());

        batch.add(
            PathBuf::from("/tmp/test.png"),
            PathBuf::from("/tmp/test.dds"),
            DdsFormat::BC7,
            1024,
            1024,
            false,
        );

        // 1024*1024*4 = 4MB > 1MB budget
        assert!(batch.should_flush());
    }

    #[test]
    fn test_batch_empty() {
        let batch = EncodeBatch::new(1024);
        assert!(batch.is_empty());
        assert_eq!(batch.len(), 0);
    }

    #[test]
    fn test_dds_format_mapping() {
        assert_eq!(
            dds_format_to_image_format(DdsFormat::BC7),
            ImageFormat::BC7RgbaUnorm
        );
        assert_eq!(
            dds_format_to_image_format(DdsFormat::BC6H),
            ImageFormat::BC6hRgbUfloat
        );
        assert_eq!(
            dds_format_to_image_format(DdsFormat::BC1),
            ImageFormat::BC1RgbaUnorm
        );
        assert_eq!(
            dds_format_to_image_format(DdsFormat::Rgba8),
            ImageFormat::Rgba8Unorm
        );
    }

    #[test]
    fn test_parse_vram_gb() {
        assert_eq!(parse_vram_string("8 GB"), 8 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_parse_vram_mb() {
        assert_eq!(parse_vram_string("1536 MB"), 1536 * 1024 * 1024);
    }
}
