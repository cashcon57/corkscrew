use std::path::PathBuf;

/// Returns the on-disk version of the app bundle by reading Info.plist.
/// This can differ from the running version if an update was just installed.
#[tauri::command]
pub fn get_installed_app_version() -> Result<String, String> {
    let app_path = get_app_bundle_path().map_err(|e| e.to_string())?;
    let plist_path = app_path.join("Contents/Info.plist");
    read_plist_version(&plist_path).map_err(|e| e.to_string())
}

/// Performs a manual self-update by downloading, extracting, and replacing the app bundle.
/// This bypasses the Tauri updater's install mechanism which has known issues on macOS.
#[tauri::command]
pub async fn manual_self_update(url: String, expected_version: String) -> Result<String, String> {
    log::info!(
        "[self_update] Starting manual update to v{} from {}",
        expected_version,
        url
    );

    let app_path = get_app_bundle_path().map_err(|e| format!("Failed to determine app path: {e}"))?;
    log::info!("[self_update] App bundle path: {}", app_path.display());

    // Step 1: Download the tar.gz
    log::info!("[self_update] Step 1: Downloading update archive...");
    let bytes = download_update(&url)
        .await
        .map_err(|e| format!("Download failed: {e}"))?;
    log::info!(
        "[self_update] Downloaded {} bytes",
        bytes.len()
    );

    // Step 2: Extract to temp directory
    log::info!("[self_update] Step 2: Extracting archive...");
    let tmp_extract = tempfile::Builder::new()
        .prefix("corkscrew_update_")
        .tempdir()
        .map_err(|e| format!("Failed to create temp dir: {e}"))?;

    extract_tar_gz(&bytes, tmp_extract.path())
        .map_err(|e| format!("Extraction failed: {e}"))?;

    // Verify extraction produced a valid app structure
    let extracted_plist = tmp_extract.path().join("Contents/Info.plist");
    if !extracted_plist.exists() {
        return Err("Extraction failed: no Contents/Info.plist in extracted app".into());
    }
    let extracted_version = read_plist_version(&extracted_plist)
        .map_err(|e| format!("Failed to read extracted version: {e}"))?;
    log::info!(
        "[self_update] Extracted version: {} (expected: {})",
        extracted_version,
        expected_version
    );

    // Step 3: Replace the app bundle
    log::info!("[self_update] Step 3: Replacing app bundle at {}...", app_path.display());
    replace_app_bundle(&app_path, tmp_extract.path())
        .map_err(|e| format!("Failed to replace app bundle: {e}"))?;

    // Step 4: Verify the installed version
    let installed_plist = app_path.join("Contents/Info.plist");
    let installed_version = read_plist_version(&installed_plist)
        .map_err(|e| format!("Post-install verification failed: {e}"))?;

    if installed_version != expected_version {
        return Err(format!(
            "Version mismatch after install: expected {}, got {}",
            expected_version, installed_version
        ));
    }

    log::info!(
        "[self_update] Update complete! Installed v{}",
        installed_version
    );
    Ok(installed_version)
}

fn get_app_bundle_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("current_exe failed: {e}"))?;

    // On macOS: .app/Contents/MacOS/binary → navigate up to .app
    let exe_str = exe.display().to_string();
    if exe_str.contains("Contents/MacOS") {
        let app_path = exe
            .parent() // MacOS/
            .and_then(|p| p.parent()) // Contents/
            .and_then(|p| p.parent()) // .app/
            .ok_or("Failed to navigate to .app bundle")?;
        Ok(app_path.to_path_buf())
    } else {
        Err(format!(
            "Not running from a macOS .app bundle: {}",
            exe_str
        ))
    }
}

async fn download_update(url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()).into());
    }

    let bytes = response.bytes().await?;
    Ok(bytes.to_vec())
}

fn extract_tar_gz(bytes: &[u8], dest: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use flate2::read::GzDecoder;
    use std::io::Cursor;

    let cursor = Cursor::new(bytes);
    let decoder = GzDecoder::new(cursor);
    let mut archive = tar::Archive::new(decoder);

    let mut file_count = 0u32;

    for entry in archive.entries()? {
        let mut entry = entry?;

        // Skip the first path component (e.g., "Corkscrew.app/Contents/..." → "Contents/...")
        let path = entry.path()?;
        let collected: PathBuf = path.iter().skip(1).collect();

        if collected.as_os_str().is_empty() {
            continue; // Skip root directory entry
        }

        let extraction_path = dest.join(&collected);

        if let Some(parent) = extraction_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        entry.unpack(&extraction_path)?;
        file_count += 1;
    }

    log::info!("[self_update] Extracted {} files/dirs", file_count);

    if file_count == 0 {
        return Err("Archive was empty or contained no extractable entries".into());
    }

    Ok(())
}

fn replace_app_bundle(
    app_path: &std::path::Path,
    new_contents: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create backup
    let backup_dir = tempfile::Builder::new()
        .prefix("corkscrew_backup_")
        .tempdir()?;
    let backup_path = backup_dir.path().join("Corkscrew.app.bak");

    // Step A: Move current app to backup
    log::info!(
        "[self_update] Moving current app to backup: {} → {}",
        app_path.display(),
        backup_path.display()
    );

    match std::fs::rename(app_path, &backup_path) {
        Ok(()) => {
            log::info!("[self_update] Backup created successfully");
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            // Try with shell mv command (might work for different permission scenarios)
            log::warn!(
                "[self_update] rename() permission denied, trying mv command: {}",
                e
            );
            let status = std::process::Command::new("mv")
                .arg("-f")
                .arg(app_path)
                .arg(&backup_path)
                .status()?;
            if !status.success() {
                return Err(format!(
                    "Failed to move app to backup (exit {}). Try closing the app and updating manually.",
                    status.code().unwrap_or(-1)
                )
                .into());
            }
        }
        Err(e) => {
            return Err(format!("Failed to backup current app: {e}").into());
        }
    }

    // Step B: Move new app to target
    log::info!(
        "[self_update] Moving new app to target: {} → {}",
        new_contents.display(),
        app_path.display()
    );

    match std::fs::rename(new_contents, app_path) {
        Ok(()) => {
            log::info!("[self_update] New app installed successfully");
        }
        Err(e) => {
            // Restore backup on failure
            log::error!("[self_update] Failed to install new app: {e}. Restoring backup...");
            if let Err(restore_err) = std::fs::rename(&backup_path, app_path) {
                log::error!(
                    "[self_update] CRITICAL: Failed to restore backup: {}. App may be missing at {}",
                    restore_err,
                    app_path.display()
                );
            }
            return Err(format!("Failed to install new app: {e}").into());
        }
    }

    // Step C: Remove quarantine xattr (macOS)
    let _ = std::process::Command::new("xattr")
        .arg("-dr")
        .arg("com.apple.quarantine")
        .arg(app_path)
        .status();

    // Step D: Touch the app to update modification time
    let _ = std::process::Command::new("touch")
        .arg(app_path)
        .status();

    log::info!("[self_update] App replacement complete");
    Ok(())
}

fn read_plist_version(plist_path: &std::path::Path) -> Result<String, Box<dyn std::error::Error>> {
    let output = std::process::Command::new("/usr/libexec/PlistBuddy")
        .arg("-c")
        .arg("Print :CFBundleShortVersionString")
        .arg(plist_path)
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "PlistBuddy failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
