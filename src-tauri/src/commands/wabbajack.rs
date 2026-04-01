//! Wabbajack modlist commands: parsing, installation, and progress tracking.

use crate::wabbajack;
use crate::config;
use crate::wabbajack::{ModlistSummary, ParsedModlist};
use tauri::Emitter;

// --- Wabbajack Modlists ---

#[tauri::command]
pub async fn get_wabbajack_modlists() -> Result<Vec<ModlistSummary>, String> {
    wabbajack::fetch_modlist_gallery().await
}

#[tauri::command]
pub async fn parse_wabbajack_file(file_path: String) -> Result<ParsedModlist, String> {
    tokio::task::spawn_blocking(move || {
        wabbajack::parse_wabbajack_file(std::path::Path::new(&file_path))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn check_wabbajack_cache(filename: String) -> Result<String, String> {
    // Validate filename to prevent path traversal
    let safe_name = std::path::Path::new(&filename)
        .file_name()
        .ok_or_else(|| "Invalid filename".to_string())?
        .to_string_lossy()
        .to_string();
    let download_dir = config::get_config()
        .ok()
        .and_then(|c| c.download_dir)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("corkscrew")
                .join("downloads")
        });
    let dest = download_dir.join(&safe_name);
    if dest.exists() && std::fs::metadata(&dest).map(|m| m.len() > 0).unwrap_or(false) {
        Ok(dest.to_string_lossy().to_string())
    } else {
        Err(format!("Cached file not found: {}", filename))
    }
}

#[tauri::command]
pub async fn download_wabbajack_file(app: tauri::AppHandle, url: String, filename: String, force: Option<bool>) -> Result<String, String> {
    // Validate URL scheme to prevent SSRF
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(format!("Blocked unsafe URL scheme: {url}"));
    }
    // Validate filename to prevent path traversal
    let safe_name = std::path::Path::new(&filename)
        .file_name()
        .ok_or_else(|| "Invalid filename".to_string())?
        .to_string_lossy()
        .to_string();

    let download_dir = config::get_config()
        .ok()
        .and_then(|c| c.download_dir)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("corkscrew")
                .join("downloads")
        });

    std::fs::create_dir_all(&download_dir)
        .map_err(|e| format!("Failed to create download directory: {e}"))?;

    let dest = download_dir.join(&safe_name);

    // Check if file already exists (skip redownload)
    // TODO: Verify cached file hash matches modlist metadata.
    // For now, existence + non-empty is sufficient since filenames include
    // the content hash (e.g., "Tuxborn.wabbajack_b49465c4-...").
    if !force.unwrap_or(false) && dest.exists() {
        let meta = std::fs::metadata(&dest);
        if let Ok(m) = meta {
            if m.len() > 0 {
                log::info!(
                    "Using cached .wabbajack file: {} ({:.1} MB)",
                    dest.display(),
                    m.len() as f64 / 1_048_576.0
                );
                return Ok(dest.to_string_lossy().to_string());
            }
        }
    }

    let client = reqwest::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let is_wj_cdn = url.contains("authored-files.wabbajack.org")
        || url.contains("wabbajack.b-cdn.net");

    if is_wj_cdn {
        // WJ CDN uses a chunked protocol: definition.json.gz + parts/{index}
        download_wj_cdn_chunked(&client, &url, &dest, &app).await?;
    } else if url.contains("github.com") && url.contains("/releases/") {
        // GitHub releases: direct download
        download_direct(&client, &url, &dest).await?;
    } else {
        // Try direct first, fall back to CDN chunked
        match download_direct(&client, &url, &dest).await {
            Ok(()) => {}
            Err(_) => {
                download_wj_cdn_chunked(&client, &url, &dest, &app).await?;
            }
        }
    }

    Ok(dest.to_string_lossy().to_string())
}

/// Direct HTTP download (GitHub releases, etc).
pub async fn download_direct(
    client: &reqwest::Client,
    url: &str,
    dest: &std::path::Path,
) -> Result<(), String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read download: {e}"))?;

    std::fs::write(dest, &bytes).map_err(|e| format!("Failed to save file: {e}"))?;
    Ok(())
}

/// WJ CDN chunked download: fetch definition.json.gz, then download parts.
///
/// The WJ CDN doesn't serve files directly. Instead:
/// 1. GET {url}/definition.json.gz → decompress → FileDefinition JSON
/// 2. For each part: GET {url}/parts/{index} → write at offset
pub async fn download_wj_cdn_chunked(
    client: &reqwest::Client,
    base_url: &str,
    dest: &std::path::Path,
    app: &tauri::AppHandle,
) -> Result<(), String> {
    // URL-encode the path portion (spaces in filenames)
    let encoded_url = base_url.replace(' ', "%20");
    let def_url = format!("{}/definition.json.gz", encoded_url);

    log::info!("Fetching WJ CDN definition from {}", def_url);

    let resp = client
        .get(&def_url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch CDN definition: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "CDN definition returned HTTP {} — the modlist may have been removed.",
            resp.status()
        ));
    }

    let compressed = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read CDN definition: {e}"))?;

    // Decompress gzip
    use std::io::Read;
    let mut decoder = flate2::read::GzDecoder::new(&compressed[..]);
    let mut json_str = String::new();
    decoder
        .read_to_string(&mut json_str)
        .map_err(|e| format!("Failed to decompress CDN definition: {e}"))?;

    let definition: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| format!("Failed to parse CDN definition: {e}"))?;

    let total_size = definition
        .get("Size")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let parts = definition
        .get("Parts")
        .and_then(|v| v.as_array())
        .ok_or("CDN definition missing Parts array")?;

    log::info!(
        "WJ CDN download: {} parts, {:.1} MB total",
        parts.len(),
        total_size as f64 / 1_048_576.0
    );

    let _ = app.emit("wj-file-download-progress", serde_json::json!({
        "phase": "started",
        "total_parts": parts.len(),
        "total_bytes": total_size,
        "current_part": 0,
        "bytes_downloaded": 0u64,
    }));

    // Write to a temp file, rename on success to avoid partial corruption
    let dest_tmp = dest.with_extension("wabbajack.part");

    // Pre-allocate the temp output file and open once before the loop
    use std::io::{Seek, SeekFrom, Write};
    let mut file = {
        let f = std::fs::File::create(&dest_tmp)
            .map_err(|e| format!("Failed to create temp output file: {e}"))?;
        f.set_len(total_size)
            .map_err(|e| format!("Failed to pre-allocate file: {e}"))?;
        drop(f);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&dest_tmp)
            .map_err(|e| format!("Failed to open temp output file: {e}"))?
    };

    // Download parts sequentially (they must be written at correct offsets)
    let download_result: Result<(), String> = async {
        for part in parts {
            let index = part.get("Index").and_then(|v| v.as_u64()).unwrap_or(0);
            let offset = part.get("Offset").and_then(|v| v.as_u64()).unwrap_or(0);
            let size = part.get("Size").and_then(|v| v.as_u64()).unwrap_or(0);

            let part_url = format!("{}/parts/{}", encoded_url, index);

            let part_resp = client
                .get(&part_url)
                .send()
                .await
                .map_err(|e| format!("Failed to download part {index}: {e}"))?;

            if !part_resp.status().is_success() {
                return Err(format!(
                    "CDN part {index} returned HTTP {}",
                    part_resp.status()
                ));
            }

            let part_bytes = part_resp
                .bytes()
                .await
                .map_err(|e| format!("Failed to read part {index}: {e}"))?;

            if part_bytes.len() as u64 != size {
                log::warn!(
                    "Part {index} size mismatch: expected {size}, got {}",
                    part_bytes.len()
                );
            }

            // Write at correct offset
            file.seek(SeekFrom::Start(offset))
                .map_err(|e| format!("Failed to seek: {e}"))?;
            file.write_all(&part_bytes)
                .map_err(|e| format!("Failed to write part {index}: {e}"))?;

            let bytes_so_far = offset + size;
            if index % 10 == 0 || index == parts.len() as u64 - 1 {
                log::info!("CDN download progress: part {}/{} ({:.1}%)", index + 1, parts.len(),
                    bytes_so_far as f64 / total_size.max(1) as f64 * 100.0);
            }
            let _ = app.emit("wj-file-download-progress", serde_json::json!({
                "phase": "downloading",
                "total_parts": parts.len(),
                "total_bytes": total_size,
                "current_part": index + 1,
                "bytes_downloaded": bytes_so_far,
            }));
        }
        Ok(())
    }.await;

    // Close the file handle
    drop(file);

    // On error, clean up the temp file
    if let Err(e) = &download_result {
        let _ = std::fs::remove_file(&dest_tmp);
        return Err(e.clone());
    }

    // Rename temp file to final destination
    std::fs::rename(&dest_tmp, dest)
        .map_err(|e| format!("Failed to rename temp file to final destination: {e}"))?;

    let _ = app.emit("wj-file-download-progress", serde_json::json!({
        "phase": "completed",
        "total_parts": parts.len(),
        "total_bytes": total_size,
        "current_part": parts.len(),
        "bytes_downloaded": total_size,
    }));

    log::info!(
        "WJ CDN download complete: {} -> {}",
        base_url,
        dest.display()
    );

    Ok(())
}

