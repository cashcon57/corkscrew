use crate::wabbajack_types::*;
use log::{debug, info, warn};
use rayon::prelude::*;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum WjDirectiveError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Archive not found for hash: {0}")]
    ArchiveNotFound(String),
    #[error("File not found in archive: {0}")]
    FileNotFound(String),
    #[error("Hash mismatch for {path}: expected {expected}, got {actual}")]
    HashMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("Patch failed: {0}")]
    PatchFailed(String),
    #[error("BSA creation failed: {0}")]
    BsaFailed(String),
    #[error("Texture transform failed: {0}")]
    TextureFailed(String),
    #[error("ZIP error: {0}")]
    ZipError(String),
    #[error("Path traversal blocked: {0}")]
    PathTraversal(String),
    #[error("{0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// Result type for batch processing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct WjDirectiveResult {
    pub total_processed: usize,
    pub total_skipped: usize,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// Directive processor
// ---------------------------------------------------------------------------

pub struct DirectiveProcessor {
    /// Path to the .wabbajack ZIP file (for reading inline data and patches)
    wabbajack_path: PathBuf,
    /// Map of archive hash → path to extracted directory
    archive_dirs: HashMap<String, PathBuf>,
    /// Output directory where files get placed
    output_dir: PathBuf,
    /// Game installation directory (for GAMEDIR substitution)
    game_dir: PathBuf,
}

impl DirectiveProcessor {
    pub fn new(
        wabbajack_path: PathBuf,
        archive_dirs: HashMap<String, PathBuf>,
        output_dir: PathBuf,
        game_dir: PathBuf,
    ) -> Self {
        DirectiveProcessor {
            wabbajack_path,
            archive_dirs,
            output_dir,
            game_dir,
        }
    }

    // -----------------------------------------------------------------------
    // Main dispatch
    // -----------------------------------------------------------------------

    /// Process a single directive by dispatching to the appropriate handler.
    pub fn process_directive(&self, directive: &WjDirective) -> Result<(), WjDirectiveError> {
        match directive {
            WjDirective::FromArchive {
                to,
                archive_hash_path,
                ..
            } => self.process_from_archive(to, archive_hash_path),

            WjDirective::PatchedFromArchive {
                hash,
                to,
                archive_hash_path,
                patch_id,
                ..
            } => self.process_patched_from_archive(to, archive_hash_path, *patch_id, hash),

            WjDirective::InlineFile {
                to, source_data_id, ..
            } => self.process_inline_file(to, *source_data_id),

            WjDirective::RemappedInlineFile {
                to, source_data_id, ..
            } => self.process_remapped_inline_file(to, *source_data_id),

            WjDirective::CreateBSA {
                to,
                temp_id,
                state,
                file_states,
                ..
            } => self.process_create_bsa(to, *temp_id, state.as_ref(), file_states),

            WjDirective::TransformedTexture {
                to,
                archive_hash_path,
                image_state,
                ..
            } => self.process_transformed_texture(to, archive_hash_path, image_state.as_ref()),

            WjDirective::MergedPatch {
                hash,
                to,
                patch_id,
                sources,
                ..
            } => self.process_merged_patch(to, *patch_id, sources, hash),

            WjDirective::IgnoredDirectly { to, reason, .. } => {
                self.process_ignored(to, reason);
                Ok(())
            }
        }
    }

    /// Process all directives in the correct order with progress reporting.
    ///
    /// Execution order:
    /// 1. FromArchive, PatchedFromArchive, InlineFile, RemappedInlineFile,
    ///    TransformedTexture (file production phase) — **parallel via rayon**
    /// 2. MergedPatch (depends on produced files) — sequential
    /// 3. CreateBSA (consumes produced files into archives) — sequential
    /// 4. IgnoredDirectly (any time, no-op)
    ///
    /// Phase 1 directives write to unique output paths, so they can safely
    /// run in parallel. Phases 2 and 3 depend on Phase 1 outputs and MUST
    /// remain sequential.
    #[allow(clippy::type_complexity)]
    pub fn process_all(
        &self,
        directives: &[WjDirective],
        progress_callback: &(dyn Fn(usize, usize, &str, u64, u64, &str) + Sync),
    ) -> Result<WjDirectiveResult, WjDirectiveError> {
        // Partition directives by processing phase
        let mut phase1: Vec<&WjDirective> = Vec::new(); // File production
        let mut phase2: Vec<&WjDirective> = Vec::new(); // Merged patches
        let mut phase3: Vec<&WjDirective> = Vec::new(); // BSA creation
        let mut ignored: Vec<&WjDirective> = Vec::new();

        for d in directives {
            match d {
                WjDirective::FromArchive { .. }
                | WjDirective::PatchedFromArchive { .. }
                | WjDirective::InlineFile { .. }
                | WjDirective::RemappedInlineFile { .. }
                | WjDirective::TransformedTexture { .. } => phase1.push(d),
                WjDirective::MergedPatch { .. } => phase2.push(d),
                WjDirective::CreateBSA { .. } => phase3.push(d),
                WjDirective::IgnoredDirectly { .. } => ignored.push(d),
            }
        }

        let total = phase1.len() + phase2.len() + phase3.len() + ignored.len();
        let processed_counter = AtomicUsize::new(0);
        let bytes_processed = AtomicU64::new(0);
        let total_bytes: u64 = directives.iter().map(|d| d.size().max(0) as u64).sum();
        let mut skipped: usize = 0;
        let warnings: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let mut errors: Vec<String> = Vec::new();

        // Process ignored directives (no-ops, count them immediately)
        for d in &ignored {
            if let Err(e) = self.process_directive(d) {
                warnings.lock().unwrap().push(format!(
                    "Ignored directive {} had error: {}",
                    d.to_path(),
                    e
                ));
            }
            skipped += 1;
            bytes_processed.fetch_add(d.size().max(0) as u64, Ordering::Relaxed);
            let count = processed_counter.fetch_add(1, Ordering::Relaxed) + 1;
            progress_callback(
                count,
                total,
                "ignored",
                bytes_processed.load(Ordering::Relaxed),
                total_bytes,
                d.to_path(),
            );
        }

        // Phase 1: File production — parallel via rayon
        let phase1_warnings = Mutex::new(Vec::new());
        let phase1_errors = Mutex::new(Vec::new());
        let num_threads = std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(4);

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .build()
            .unwrap_or_else(|_| {
                rayon::ThreadPoolBuilder::new()
                    .build()
                    .expect("failed to build default rayon pool")
            });

        pool.install(|| {
            phase1.par_iter().for_each(|d| {
                if let Err(e) = self.process_directive(d) {
                    let msg = format!("{} -> {}: {}", d.kind_name(), d.to_path(), e);
                    // Hash mismatches are non-fatal warnings — the file was still written
                    if matches!(e, WjDirectiveError::HashMismatch { .. }) {
                        warn!("Directive warning: {}", msg);
                        phase1_warnings.lock().unwrap().push(msg);
                    } else {
                        warn!("Directive error: {}", msg);
                        phase1_errors.lock().unwrap().push(msg);
                    }
                }

                bytes_processed.fetch_add(d.size().max(0) as u64, Ordering::Relaxed);
                let count = processed_counter.fetch_add(1, Ordering::Relaxed) + 1;
                if count.is_multiple_of(5) || count == total || count == phase1.len() {
                    progress_callback(
                        count,
                        total,
                        "files",
                        bytes_processed.load(Ordering::Relaxed),
                        total_bytes,
                        d.to_path(),
                    );
                }
            });
        });

        errors.extend(phase1_errors.into_inner().unwrap());
        warnings
            .lock()
            .unwrap()
            .extend(phase1_warnings.into_inner().unwrap());

        // Phase 2: Merged patches (sequential — depends on Phase 1 outputs)
        for d in &phase2 {
            match self.process_directive(d) {
                Ok(()) => {}
                Err(e) => {
                    let msg = format!("{} -> {}: {}", d.kind_name(), d.to_path(), e);
                    warn!("Directive error: {}", msg);
                    errors.push(msg);
                }
            }
            bytes_processed.fetch_add(d.size().max(0) as u64, Ordering::Relaxed);
            let count = processed_counter.fetch_add(1, Ordering::Relaxed) + 1;
            progress_callback(
                count,
                total,
                "patches",
                bytes_processed.load(Ordering::Relaxed),
                total_bytes,
                d.to_path(),
            );
        }

        // Phase 3: BSA creation (sequential — consumes produced files)
        for d in &phase3 {
            match self.process_directive(d) {
                Ok(()) => {}
                Err(e) => {
                    let msg = format!("{} -> {}: {}", d.kind_name(), d.to_path(), e);
                    warn!("Directive error: {}", msg);
                    errors.push(msg);
                }
            }
            bytes_processed.fetch_add(d.size().max(0) as u64, Ordering::Relaxed);
            let count = processed_counter.fetch_add(1, Ordering::Relaxed) + 1;
            progress_callback(
                count,
                total,
                "archives",
                bytes_processed.load(Ordering::Relaxed),
                total_bytes,
                d.to_path(),
            );
        }

        let processed = processed_counter.load(Ordering::Relaxed);

        Ok(WjDirectiveResult {
            total_processed: processed - skipped - errors.len(),
            total_skipped: skipped,
            warnings: warnings.into_inner().unwrap(),
            errors,
        })
    }

    // -----------------------------------------------------------------------
    // Individual directive handlers
    // -----------------------------------------------------------------------

    /// Extract a file from a downloaded archive and copy to output.
    ///
    /// Parses the `ArchiveHashPath` to find the archive extraction directory
    /// by base hash, then locates the file at the relative path within.
    fn process_from_archive(
        &self,
        to: &str,
        archive_hash_path: &ArchiveHashPath,
    ) -> Result<(), WjDirectiveError> {
        let source_path = self.resolve_archive_file(archive_hash_path)?;
        let dest_path = self.resolve_output_path(to)?;
        ensure_parent_dir(&dest_path)?;
        std::fs::copy(&source_path, &dest_path).map_err(|e| {
            WjDirectiveError::Io(std::io::Error::new(
                e.kind(),
                format!(
                    "Failed to copy {} -> {}: {}",
                    source_path.display(),
                    dest_path.display(),
                    e
                ),
            ))
        })?;
        debug!("FromArchive: {} -> {}", source_path.display(), to);
        Ok(())
    }

    /// Extract source file from archive, apply a BSDiff patch from the
    /// .wabbajack ZIP, and write the patched result to output.
    fn process_patched_from_archive(
        &self,
        to: &str,
        archive_hash_path: &ArchiveHashPath,
        patch_id: i64,
        expected_hash: &WjHash,
    ) -> Result<(), WjDirectiveError> {
        // Read the source file from the extracted archive
        let source_path = self.resolve_archive_file(archive_hash_path)?;
        let source_data = std::fs::read(&source_path).map_err(|e| {
            WjDirectiveError::Io(std::io::Error::new(
                e.kind(),
                format!("Failed to read source {}: {}", source_path.display(), e),
            ))
        })?;

        // Read the patch data from the .wabbajack ZIP
        let patch_entry_name = patch_id.to_string();
        let patch_data = read_wj_zip_entry(&self.wabbajack_path, &patch_entry_name)?;

        // Apply BSDiff patch
        let patcher = qbsdiff::Bspatch::new(&patch_data)
            .map_err(|e| WjDirectiveError::PatchFailed(format!("Invalid patch data: {}", e)))?;

        let target_size = patcher.hint_target_size() as usize;
        let mut target_data = Vec::with_capacity(target_size);

        patcher
            .apply(&source_data, &mut target_data)
            .map_err(|e| WjDirectiveError::PatchFailed(format!("Patch apply failed: {}", e)))?;

        // Write output
        let dest_path = self.resolve_output_path(to)?;
        ensure_parent_dir(&dest_path)?;
        std::fs::write(&dest_path, &target_data)?;

        // Verify hash if not empty
        if !expected_hash.is_empty() {
            verify_output_hash(&dest_path, expected_hash)?;
        }

        debug!(
            "PatchedFromArchive: {} (patch {}) -> {}",
            source_path.display(),
            patch_id,
            to
        );
        Ok(())
    }

    /// Read an inline file from the .wabbajack ZIP and write to output.
    fn process_inline_file(&self, to: &str, source_data_id: i64) -> Result<(), WjDirectiveError> {
        let entry_name = source_data_id.to_string();
        let data = read_wj_zip_entry(&self.wabbajack_path, &entry_name)?;

        let dest_path = self.resolve_output_path(to)?;
        ensure_parent_dir(&dest_path)?;
        std::fs::write(&dest_path, &data)?;

        debug!("InlineFile: entry {} -> {}", source_data_id, to);
        Ok(())
    }

    /// Read an inline file from the .wabbajack ZIP, apply text substitutions
    /// (GAMEDIR, MO2DIR), and write to output.
    fn process_remapped_inline_file(
        &self,
        to: &str,
        source_data_id: i64,
    ) -> Result<(), WjDirectiveError> {
        let entry_name = source_data_id.to_string();
        let data = read_wj_zip_entry(&self.wabbajack_path, &entry_name)?;

        // Attempt to treat as text for substitution
        let text = String::from_utf8_lossy(&data);
        let remapped = substitute_wj_path(&text, &self.game_dir, &self.output_dir);

        let dest_path = self.resolve_output_path(to)?;
        ensure_parent_dir(&dest_path)?;
        std::fs::write(&dest_path, remapped.as_bytes())?;

        debug!("RemappedInlineFile: entry {} -> {}", source_data_id, to);
        Ok(())
    }

    /// Create a BSA/BA2 archive from files produced by earlier directives.
    ///
    /// Determines archive format from BsaState.$type:
    /// - Contains "BA2" -> Fallout 4 BA2 format (GNRL or DX10)
    /// - Otherwise -> TES4 BSA format (Skyrim SE v105 by default)
    ///
    /// Each BsaFileState.path references a file that was previously placed in
    /// the output directory. We read those files, pack them into the archive,
    /// and write the result to `dest_path`.
    fn process_create_bsa(
        &self,
        to: &str,
        temp_id: i64,
        state: Option<&BsaState>,
        file_states: &[BsaFileState],
    ) -> Result<(), WjDirectiveError> {
        let dest_path = self.resolve_output_path(to)?;
        ensure_parent_dir(&dest_path)?;

        if file_states.is_empty() {
            warn!(
                "CreateBSA: {} -> {} has no file states, skipping",
                temp_id, to
            );
            return Ok(());
        }

        // Determine archive format from state_type
        let is_ba2 = state
            .map(|s| {
                let t = s.state_type.to_uppercase();
                t.contains("BA2")
            })
            .unwrap_or(false);

        // Determine if this is a texture (DX10) archive
        let is_dx10 = state
            .map(|s| {
                let t = s.state_type.to_uppercase();
                t.contains("DX10") || t.contains("TEXTURE")
            })
            .unwrap_or(false);

        info!(
            "CreateBSA: packing {} files into {} (format: {})",
            file_states.len(),
            to,
            if is_ba2 {
                if is_dx10 {
                    "BA2/DX10"
                } else {
                    "BA2/GNRL"
                }
            } else {
                "BSA/TES4"
            }
        );

        if is_ba2 {
            self.pack_ba2_archive(&dest_path, file_states, is_dx10, temp_id)?;
        } else {
            self.pack_bsa_archive(&dest_path, file_states, state, temp_id)?;
        }

        // Clean up any leftover staging directory from previous runs
        let staging_dir = dest_path.with_extension("bsa_staging");
        if staging_dir.exists() {
            std::fs::remove_dir_all(&staging_dir).ok();
        }

        debug!(
            "CreateBSA: {} -> {} ({} files packed)",
            temp_id,
            to,
            file_states.len()
        );
        Ok(())
    }

    /// Pack files into a Fallout 4 BA2 archive (GNRL or DX10 format).
    ///
    /// Uses the `ba2` crate's fo4 module. For DX10 (texture) BA2 archives,
    /// we parse each DDS file's header to extract texture metadata (width,
    /// height, mip count, DXGI format) and construct properly structured
    /// DX10 file entries with mip chunk info. Non-DDS files in a DX10 archive
    /// fall back to GNRL-style packing with a warning.
    fn pack_ba2_archive(
        &self,
        dest_path: &Path,
        file_states: &[BsaFileState],
        is_dx10: bool,
        temp_id: i64,
    ) -> Result<(), WjDirectiveError> {
        use ba2::fo4::{
            Archive, ArchiveKey, ArchiveOptions, Chunk, DX10Header, File, FileHeader, Format,
            Version,
        };
        use ba2::CompressableFrom;

        let mut archive = Archive::new();

        let canonical_output = self
            .output_dir
            .canonicalize()
            .unwrap_or_else(|_| self.output_dir.clone());

        for fs in file_states {
            let normalized = normalize_wj_path(&fs.path);
            // Files are in the output directory, placed there by earlier directives
            let file_path = self.output_dir.join(&normalized);

            // Path traversal check for BA2 file states
            if let Ok(canonical_fp) = file_path.canonicalize() {
                if !canonical_fp.starts_with(&canonical_output) {
                    return Err(WjDirectiveError::PathTraversal(format!(
                        "BA2 file state path {} escapes output directory",
                        canonical_fp.display()
                    )));
                }
            }

            let resolved_path = if file_path.exists() {
                file_path.clone()
            } else {
                // Try case-insensitive lookup
                let rel = PathBuf::from(&normalized);
                if let Some(found) = case_insensitive_find(&self.output_dir, &rel) {
                    found
                } else {
                    warn!(
                        "CreateBSA(BA2): file not found for packing: {} (temp_id={})",
                        file_path.display(),
                        temp_id
                    );
                    continue;
                }
            };

            let data = std::fs::read(&resolved_path).map_err(|e| {
                WjDirectiveError::BsaFailed(format!(
                    "Failed to read {}: {}",
                    resolved_path.display(),
                    e
                ))
            })?;

            let file = if is_dx10 {
                // For DX10 archives, try to parse DDS header for texture metadata
                if let Some(dds_info) = parse_dds_header(&data) {
                    let dx10_header = DX10Header {
                        height: dds_info.height,
                        width: dds_info.width,
                        mip_count: dds_info.mip_count,
                        format: dds_info.dxgi_format,
                        flags: if dds_info.is_cubemap { 1 } else { 0 },
                        tile_mode: 8, // Standard linear tile mode
                    };

                    // Strip DDS header, pack only the pixel data
                    if dds_info.data_offset > data.len() {
                        warn!(
                            "DDS data_offset ({}) exceeds file size ({}), falling back to GNRL",
                            dds_info.data_offset,
                            data.len()
                        );
                        // Fall through to GNRL-style packing below
                        let chunk = Chunk::from_decompressed(data.into_boxed_slice());
                        let file: File = std::iter::once(chunk).collect();
                        let key = ArchiveKey::from(normalized.replace('/', "\\").as_bytes());
                        archive.insert(key, file);
                        continue;
                    }
                    let pixel_data = data[dds_info.data_offset..].to_vec();
                    let mip_range = 0..=(dds_info.mip_count.saturating_sub(1) as u16);
                    let mut chunk = Chunk::from_decompressed(pixel_data.into_boxed_slice());
                    chunk.mips = Some(mip_range);

                    let mut file = File::new();
                    file.header = FileHeader::DX10(dx10_header);
                    file.push(chunk);
                    file
                } else {
                    // Not a valid DDS or unrecognized format -- pack as GNRL-style
                    // within the DX10 archive. This shouldn't normally happen but
                    // handles edge cases gracefully.
                    warn!(
                        "CreateBSA(BA2/DX10): {} is not a valid DDS file, \
                         packing as raw data (temp_id={})",
                        resolved_path.display(),
                        temp_id
                    );
                    let chunk = Chunk::from_decompressed(data.into_boxed_slice());
                    std::iter::once(chunk).collect()
                }
            } else {
                // GNRL format: pack raw file data as-is
                let chunk = Chunk::from_decompressed(data.into_boxed_slice());
                std::iter::once(chunk).collect()
            };

            // BA2 paths use backslash-separated paths
            let key_path = normalized.replace('/', "\\");
            let key = ArchiveKey::from(key_path.as_bytes());
            archive.insert(key, file);
        }

        // Write BA2 with the appropriate format, version 1
        let format = if is_dx10 { Format::DX10 } else { Format::GNRL };
        let options = ArchiveOptions::builder()
            .format(format)
            .version(Version::v1)
            .build();

        let mut output = std::fs::File::create(dest_path).map_err(|e| {
            WjDirectiveError::BsaFailed(format!(
                "Failed to create BA2 {}: {}",
                dest_path.display(),
                e
            ))
        })?;

        archive.write(&mut output, &options).map_err(|e| {
            WjDirectiveError::BsaFailed(format!(
                "Failed to write BA2 {}: {}",
                dest_path.display(),
                e
            ))
        })?;

        Ok(())
    }

    /// Pack files into a TES4 BSA archive (Skyrim/Oblivion/Fallout3 format).
    ///
    /// Uses the `ba2` crate's tes4 module. Determines the BSA version from
    /// the BsaState type string and guesses ArchiveTypes from file extensions.
    fn pack_bsa_archive(
        &self,
        dest_path: &Path,
        file_states: &[BsaFileState],
        state: Option<&BsaState>,
        temp_id: i64,
    ) -> Result<(), WjDirectiveError> {
        use ba2::tes4::{
            Archive, ArchiveFlags, ArchiveKey, ArchiveOptions, ArchiveTypes, Directory,
            DirectoryKey, File, Version,
        };
        use ba2::CompressableFrom;

        // Determine BSA version from state type
        let version = state
            .map(|s| {
                let t = s.state_type.to_uppercase();
                if t.contains("SSE") || t.contains("SE") || t.contains("105") {
                    Version::v105
                } else if t.contains("FO3")
                    || t.contains("FNV")
                    || t.contains("SKYRIM")
                    || t.contains("104")
                {
                    Version::v104
                } else if t.contains("OBLIVION") || t.contains("103") {
                    Version::v103
                } else {
                    // Default to SSE for modern modlists
                    Version::v105
                }
            })
            .unwrap_or(Version::v105);

        // Group files by their parent directory within the BSA
        let mut dir_map: HashMap<String, Vec<(String, Vec<u8>)>> = HashMap::new();
        let mut archive_types = ArchiveTypes::empty();
        let canonical_output = self
            .output_dir
            .canonicalize()
            .unwrap_or_else(|_| self.output_dir.clone());

        for fs in file_states {
            let normalized = normalize_wj_path(&fs.path);
            let file_path = self.output_dir.join(&normalized);

            // Path traversal check for BSA file states
            if let Ok(canonical_fp) = file_path.canonicalize() {
                if !canonical_fp.starts_with(&canonical_output) {
                    return Err(WjDirectiveError::PathTraversal(format!(
                        "BSA file state path {} escapes output directory",
                        canonical_fp.display()
                    )));
                }
            }

            let data = if file_path.exists() {
                std::fs::read(&file_path).map_err(|e| {
                    WjDirectiveError::BsaFailed(format!(
                        "Failed to read {}: {}",
                        file_path.display(),
                        e
                    ))
                })?
            } else {
                // Try case-insensitive lookup
                let rel = PathBuf::from(&normalized);
                if let Some(found) = case_insensitive_find(&self.output_dir, &rel) {
                    std::fs::read(&found).map_err(|e| {
                        WjDirectiveError::BsaFailed(format!(
                            "Failed to read {}: {}",
                            found.display(),
                            e
                        ))
                    })?
                } else {
                    warn!(
                        "CreateBSA(BSA): file not found for packing: {} (temp_id={})",
                        file_path.display(),
                        temp_id
                    );
                    continue;
                }
            };

            // Detect archive type from file extension
            let ext = std::path::Path::new(&normalized)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase());
            match ext.as_deref() {
                Some("nif") | Some("btr") | Some("bto") => {
                    archive_types |= ArchiveTypes::MESHES;
                }
                Some("dds") | Some("tga") | Some("png") => {
                    archive_types |= ArchiveTypes::TEXTURES;
                }
                Some("wav") | Some("xwm") | Some("fuz") => {
                    archive_types |= ArchiveTypes::SOUNDS;
                }
                Some("lip") => {
                    archive_types |= ArchiveTypes::VOICES;
                }
                Some("swf") | Some("txt") => {
                    archive_types |= ArchiveTypes::MENUS;
                }
                _ => {
                    // Check for voice directory as a secondary heuristic
                    if normalized.to_lowercase().contains("voice") {
                        archive_types |= ArchiveTypes::VOICES;
                    } else {
                        archive_types |= ArchiveTypes::MISC;
                    }
                }
            }

            // BSA paths use backslashes. Split into directory + filename.
            let bsa_path = normalized.replace('/', "\\");
            let (dir_part, file_part) = if let Some(pos) = bsa_path.rfind('\\') {
                (bsa_path[..pos].to_string(), bsa_path[pos + 1..].to_string())
            } else {
                // File at root level
                (String::new(), bsa_path)
            };

            dir_map.entry(dir_part).or_default().push((file_part, data));
        }

        // Build the archive from grouped directories
        let mut archive = Archive::new();
        for (dir_name, files) in &dir_map {
            let mut directory = Directory::new();
            for (file_name, data) in files {
                let file = File::from_decompressed(data.as_slice());
                let key = DirectoryKey::from(file_name.as_bytes());
                directory.insert(key, file);
            }
            let archive_key = ArchiveKey::from(dir_name.as_bytes());
            archive.insert(archive_key, directory);
        }

        // Build options with appropriate flags
        let flags = ArchiveFlags::DIRECTORY_STRINGS | ArchiveFlags::FILE_STRINGS;
        let options = ArchiveOptions::builder()
            .version(version)
            .types(if archive_types.is_empty() {
                ArchiveTypes::MISC
            } else {
                archive_types
            })
            .flags(flags)
            .build();

        let mut output = std::fs::File::create(dest_path).map_err(|e| {
            WjDirectiveError::BsaFailed(format!(
                "Failed to create BSA {}: {}",
                dest_path.display(),
                e
            ))
        })?;

        archive.write(&mut output, &options).map_err(|e| {
            WjDirectiveError::BsaFailed(format!(
                "Failed to write BSA {}: {}",
                dest_path.display(),
                e
            ))
        })?;

        Ok(())
    }

    /// Extract a texture from an archive, transform it (resize, reformat),
    /// and write to output as a DDS file.
    ///
    /// If `image_state` is provided, the source DDS is decoded to RGBA,
    /// resized to the target dimensions, re-encoded to the target DDS
    /// format, and written out. If no `image_state` is given, or if any
    /// transformation step fails, the source texture is copied unchanged
    /// as a fallback.
    fn process_transformed_texture(
        &self,
        to: &str,
        archive_hash_path: &ArchiveHashPath,
        image_state: Option<&ImageState>,
    ) -> Result<(), WjDirectiveError> {
        let source_path = self.resolve_archive_file(archive_hash_path)?;
        let dest_path = self.resolve_output_path(to)?;
        ensure_parent_dir(&dest_path)?;

        // If no image state or zero dimensions, just copy as-is
        let img_state = match image_state {
            Some(s) if s.width > 0 && s.height > 0 => s,
            _ => {
                std::fs::copy(&source_path, &dest_path).map_err(|e| {
                    WjDirectiveError::TextureFailed(format!(
                        "Failed to copy texture {} -> {}: {}",
                        source_path.display(),
                        dest_path.display(),
                        e
                    ))
                })?;
                debug!(
                    "TransformedTexture: {} -> {} (no image state, copied as-is)",
                    source_path.display(),
                    to
                );
                return Ok(());
            }
        };

        // Attempt DDS transformation; fall back to copy on any failure
        match self.transform_dds(&source_path, &dest_path, img_state) {
            Ok(()) => {
                debug!(
                    "TransformedTexture: {} -> {} ({}x{}, DXGI format {})",
                    source_path.display(),
                    to,
                    img_state.width,
                    img_state.height,
                    img_state.format,
                );
            }
            Err(e) => {
                warn!(
                    "TransformedTexture: DDS transform failed for {} -> {}: {}. Copying unchanged.",
                    source_path.display(),
                    to,
                    e
                );
                std::fs::copy(&source_path, &dest_path).map_err(|e| {
                    WjDirectiveError::TextureFailed(format!(
                        "Fallback copy failed {} -> {}: {}",
                        source_path.display(),
                        dest_path.display(),
                        e
                    ))
                })?;
            }
        }

        Ok(())
    }

    /// Perform the actual DDS texture transformation: decode, resize, re-encode.
    ///
    /// Maps DXGI_FORMAT values to image_dds ImageFormat variants:
    ///   71 = BC1_UNorm, 77 = BC3_UNorm, 80 = BC4_UNorm,
    ///   83 = BC5_UNorm, 87 = B8G8R8A8_UNorm, 98 = BC7_UNorm
    fn transform_dds(
        &self,
        source_path: &Path,
        dest_path: &Path,
        img_state: &ImageState,
    ) -> Result<(), WjDirectiveError> {
        use image_dds::{Mipmaps, Quality};

        // Read the source DDS file
        let source_data = std::fs::read(source_path).map_err(|e| {
            WjDirectiveError::TextureFailed(format!(
                "Failed to read DDS {}: {}",
                source_path.display(),
                e
            ))
        })?;

        let dds = ddsfile::Dds::read(&mut Cursor::new(&source_data)).map_err(|e| {
            WjDirectiveError::TextureFailed(format!(
                "Failed to parse DDS {}: {}",
                source_path.display(),
                e
            ))
        })?;

        // Decode to RGBA image (mip level 0)
        let rgba_image = image_dds::image_from_dds(&dds, 0).map_err(|e| {
            WjDirectiveError::TextureFailed(format!(
                "Failed to decode DDS {}: {}",
                source_path.display(),
                e
            ))
        })?;

        // Resize to target dimensions using Lanczos3
        let resized = image::imageops::resize(
            &rgba_image,
            img_state.width,
            img_state.height,
            image::imageops::FilterType::Lanczos3,
        );

        // Map DXGI_FORMAT u32 to image_dds ImageFormat
        let target_format = dxgi_to_image_format(img_state.format);

        // Determine mipmap generation
        let mipmaps = if img_state.mip_levels > 1 {
            Mipmaps::GeneratedExact(img_state.mip_levels)
        } else {
            Mipmaps::Disabled
        };

        // Re-encode to DDS
        let new_dds = image_dds::dds_from_image(&resized, target_format, Quality::Normal, mipmaps)
            .map_err(|e| {
                WjDirectiveError::TextureFailed(format!(
                    "Failed to encode DDS (format {:?}): {}",
                    target_format, e
                ))
            })?;

        // Write the DDS to destination
        let mut output_file = std::fs::File::create(dest_path).map_err(|e| {
            WjDirectiveError::TextureFailed(format!(
                "Failed to create DDS output {}: {}",
                dest_path.display(),
                e
            ))
        })?;

        new_dds.write(&mut output_file).map_err(|e| {
            WjDirectiveError::TextureFailed(format!(
                "Failed to write DDS {}: {}",
                dest_path.display(),
                e
            ))
        })?;

        Ok(())
    }

    /// Merge multiple source files via a patch.
    ///
    /// Simplified implementation: reads the first source file and applies
    /// the patch from the .wabbajack ZIP. Full implementation would
    /// concatenate or merge all sources before patching.
    fn process_merged_patch(
        &self,
        to: &str,
        patch_id: i64,
        sources: &[SourcePatch],
        expected_hash: &WjHash,
    ) -> Result<(), WjDirectiveError> {
        if sources.is_empty() {
            return Err(WjDirectiveError::PatchFailed(
                "MergedPatch has no sources".to_string(),
            ));
        }

        // Concatenate ALL source files in order — Wabbajack merges all sources
        // before applying the BSDiff patch to produce the final output.
        let mut source_data = Vec::new();
        let mut missing_sources = Vec::new();
        for sp in sources {
            let source_rel = normalize_wj_path(&sp.relative_path);
            let source_path = self.output_dir.join(&source_rel);
            if source_path.exists() {
                source_data.extend(std::fs::read(&source_path)?);
            } else {
                missing_sources.push(source_rel);
            }
        }
        if !missing_sources.is_empty() {
            return Err(WjDirectiveError::PatchFailed(format!(
                "MergedPatch missing {} source(s): {}",
                missing_sources.len(),
                missing_sources.join(", ")
            )));
        }

        // Read the patch from the .wabbajack ZIP
        let patch_entry_name = patch_id.to_string();
        let patch_data = read_wj_zip_entry(&self.wabbajack_path, &patch_entry_name)?;

        // Apply BSDiff patch
        let patcher = qbsdiff::Bspatch::new(&patch_data)
            .map_err(|e| WjDirectiveError::PatchFailed(format!("Invalid merge patch: {}", e)))?;

        let target_size = patcher.hint_target_size() as usize;
        let mut target_data = Vec::with_capacity(target_size);

        patcher.apply(&source_data, &mut target_data).map_err(|e| {
            WjDirectiveError::PatchFailed(format!("Merge patch apply failed: {}", e))
        })?;

        // Write output
        let dest_path = self.resolve_output_path(to)?;
        ensure_parent_dir(&dest_path)?;
        std::fs::write(&dest_path, &target_data)?;

        // Verify hash if not empty
        if !expected_hash.is_empty() {
            verify_output_hash(&dest_path, expected_hash)?;
        }

        debug!(
            "MergedPatch: {} sources, patch {} -> {}",
            sources.len(),
            patch_id,
            to
        );
        Ok(())
    }

    /// No-op handler for ignored directives.
    fn process_ignored(&self, to: &str, reason: &str) {
        debug!("IgnoredDirectly: {} (reason: {})", to, reason);
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Resolve an ArchiveHashPath to an actual file on disk.
    ///
    /// Looks up the archive extraction directory by `base_hash`, then
    /// constructs the full path using the parts (relative path components).
    fn resolve_archive_file(&self, ahp: &ArchiveHashPath) -> Result<PathBuf, WjDirectiveError> {
        let hash_str = &ahp.base_hash.0;

        let archive_dir = self
            .archive_dirs
            .get(hash_str)
            .ok_or_else(|| WjDirectiveError::ArchiveNotFound(hash_str.clone()))?;

        // Build relative path from parts, normalizing Windows separators
        let mut rel_path = PathBuf::new();
        for part in &ahp.parts {
            rel_path.push(normalize_wj_path(part));
        }

        let full_path = archive_dir.join(&rel_path);

        if !full_path.exists() {
            // Try case-insensitive lookup as a fallback
            if let Some(found) = case_insensitive_find(archive_dir, &rel_path) {
                return Ok(found);
            }
            return Err(WjDirectiveError::FileNotFound(format!(
                "{}:{} (looked in {})",
                hash_str,
                rel_path.display(),
                archive_dir.display()
            )));
        }

        Ok(full_path)
    }

    /// Resolve a Wabbajack `To` path to an absolute output path.
    /// Rejects path traversal attempts as defense-in-depth.
    fn resolve_output_path(&self, to: &str) -> Result<PathBuf, WjDirectiveError> {
        let normalized = normalize_wj_path(to);
        if !crate::staging::is_safe_relative_path(&normalized) {
            log::warn!(
                "WJ directive has unsafe output path, sanitizing: {}",
                normalized
            );
            // Strip any dangerous components and use just the filename
            let safe = std::path::Path::new(&normalized)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            return Ok(self.output_dir.join(safe));
        }
        let dst = self.output_dir.join(&normalized);

        // ZIP Slip protection: verify the resolved path stays within output_dir
        let canonical_output = self
            .output_dir
            .canonicalize()
            .unwrap_or_else(|_| self.output_dir.clone());

        if let Ok(canonical_dst) = dst.canonicalize() {
            if !canonical_dst.starts_with(&canonical_output) {
                return Err(WjDirectiveError::PathTraversal(format!(
                    "resolved path {} escapes output directory {}",
                    canonical_dst.display(),
                    canonical_output.display()
                )));
            }
        } else if let Some(parent) = dst.parent() {
            // File doesn't exist yet — check its parent directory
            if parent.exists() {
                if let Ok(canonical_parent) = parent.canonicalize() {
                    if !canonical_parent.starts_with(&canonical_output) {
                        return Err(WjDirectiveError::PathTraversal(format!(
                            "parent path {} escapes output directory {}",
                            canonical_parent.display(),
                            canonical_output.display()
                        )));
                    }
                }
            }
        }

        Ok(dst)
    }
}

// ---------------------------------------------------------------------------
// DXGI format mapping
// ---------------------------------------------------------------------------

/// Map a DXGI_FORMAT u32 value to an image_dds ImageFormat.
///
/// Common DXGI format values used in Bethesda modding:
///   28 = R8G8B8A8_UNorm
///   71 = BC1_UNorm (DXT1)
///   72 = BC1_UNorm_sRGB
///   74 = BC2_UNorm (DXT3)
///   77 = BC3_UNorm (DXT5)
///   78 = BC3_UNorm_sRGB
///   80 = BC4_UNorm (ATI1)
///   83 = BC5_UNorm (ATI2)
///   87 = B8G8R8A8_UNorm
///   98 = BC7_UNorm
///   99 = BC7_UNorm_sRGB
fn dxgi_to_image_format(dxgi: u32) -> image_dds::ImageFormat {
    use image_dds::ImageFormat;
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
        98 => ImageFormat::BC7RgbaUnorm,
        99 => ImageFormat::BC7RgbaUnormSrgb,
        // Default to BC7 for unrecognized formats -- it handles all texture
        // types reasonably well and is the most common modern format
        other => {
            warn!("Unknown DXGI_FORMAT {}, defaulting to BC7_UNorm", other);
            ImageFormat::BC7RgbaUnorm
        }
    }
}

// ---------------------------------------------------------------------------
// DDS header parsing for BA2 DX10 texture archives
// ---------------------------------------------------------------------------

/// Metadata extracted from a DDS file header, used to construct BA2 DX10 file entries.
struct DdsHeaderInfo {
    width: u16,
    height: u16,
    mip_count: u8,
    /// DXGI_FORMAT value (for DX10 extended header) or translated from FourCC/pixel format.
    dxgi_format: u8,
    /// True if the texture is a cubemap (DDS_CUBEMAP flag set).
    is_cubemap: bool,
    /// Byte offset where the pixel data begins (after all headers).
    data_offset: usize,
}

/// Parse a DDS file's header to extract metadata needed for BA2 DX10 archives.
///
/// Supports both legacy DDS headers (with FourCC like DXT1/DXT5) and DX10-extended
/// headers (with DXGI_FORMAT). Returns `None` if the data is not a valid DDS file
/// or uses an unrecognized pixel format.
///
/// Reference: <https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dx-graphics-dds-pguide>
fn parse_dds_header(data: &[u8]) -> Option<DdsHeaderInfo> {
    // DDS files must start with "DDS " magic (0x20534444) followed by a 124-byte header
    if data.len() < 128 {
        return None;
    }

    let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if magic != 0x20534444 {
        // Not "DDS "
        return None;
    }

    // DDS_HEADER starts at offset 4
    let header = &data[4..];

    let height = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
    let width = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
    let mip_count = u32::from_le_bytes([header[24], header[25], header[26], header[27]]);
    let caps2 = u32::from_le_bytes([header[104], header[105], header[106], header[107]]);

    // DDS_HEADER.ddspf (pixel format) starts at offset 72 within the header (76 from file start)
    let pf_flags = u32::from_le_bytes([header[76], header[77], header[78], header[79]]);
    let pf_fourcc = u32::from_le_bytes([header[80], header[81], header[82], header[83]]);

    let is_cubemap = (caps2 & 0x200) != 0; // DDSCAPS2_CUBEMAP

    // DDPF_FOURCC = 0x4
    let has_fourcc = (pf_flags & 0x4) != 0;

    let (dxgi_format, data_offset) = if has_fourcc && pf_fourcc == u32::from_le_bytes(*b"DX10") {
        // DX10 extended header follows the standard 124-byte DDS header
        // DDS_HEADER_DXT10 is 20 bytes starting at offset 128
        if data.len() < 148 {
            return None;
        }
        let dxgi = u32::from_le_bytes([data[128], data[129], data[130], data[131]]);
        (dxgi as u8, 148usize)
    } else if has_fourcc {
        // Legacy FourCC: translate common formats to DXGI_FORMAT values
        let dxgi = match &pf_fourcc.to_le_bytes() {
            b"DXT1" => 71u32,        // DXGI_FORMAT_BC1_UNORM
            b"DXT3" => 74,           // DXGI_FORMAT_BC2_UNORM
            b"DXT5" => 77,           // DXGI_FORMAT_BC3_UNORM
            b"ATI1" | b"BC4U" => 80, // DXGI_FORMAT_BC4_UNORM
            b"ATI2" | b"BC5U" => 83, // DXGI_FORMAT_BC5_UNORM
            _ => {
                warn!(
                    "DDS: unrecognized FourCC {:?}, cannot determine DXGI format",
                    std::str::from_utf8(&pf_fourcc.to_le_bytes()).unwrap_or("????")
                );
                return None;
            }
        };
        (dxgi as u8, 128usize)
    } else {
        // Uncompressed format -- check for common RGBA layouts
        let rgb_bit_count = u32::from_le_bytes([header[84], header[85], header[86], header[87]]);
        let r_mask = u32::from_le_bytes([header[88], header[89], header[90], header[91]]);
        let g_mask = u32::from_le_bytes([header[92], header[93], header[94], header[95]]);
        let b_mask = u32::from_le_bytes([header[96], header[97], header[98], header[99]]);
        let a_mask = u32::from_le_bytes([header[100], header[101], header[102], header[103]]);

        let dxgi = if rgb_bit_count == 32
            && r_mask == 0x000000FF
            && g_mask == 0x0000FF00
            && b_mask == 0x00FF0000
            && a_mask == 0xFF000000
        {
            28u32 // DXGI_FORMAT_R8G8B8A8_UNORM
        } else if rgb_bit_count == 32
            && b_mask == 0x000000FF
            && g_mask == 0x0000FF00
            && r_mask == 0x00FF0000
            && a_mask == 0xFF000000
        {
            87u32 // DXGI_FORMAT_B8G8R8A8_UNORM
        } else {
            warn!(
                "DDS: unrecognized uncompressed format (bits={}, R={:#x}, G={:#x}, B={:#x}, A={:#x})",
                rgb_bit_count, r_mask, g_mask, b_mask, a_mask
            );
            return None;
        };
        (dxgi as u8, 128usize)
    };

    // Ensure mip_count is at least 1
    let mip_count = std::cmp::max(1, mip_count);

    Some(DdsHeaderInfo {
        width: width as u16,
        height: height as u16,
        mip_count: mip_count as u8,
        dxgi_format,
        is_cubemap,
        data_offset,
    })
}

// ---------------------------------------------------------------------------
// Standalone helper functions
// ---------------------------------------------------------------------------

/// Read an entry from a .wabbajack ZIP file by name.
pub fn read_wj_zip_entry(
    wabbajack_path: &Path,
    entry_name: &str,
) -> Result<Vec<u8>, WjDirectiveError> {
    let file = std::fs::File::open(wabbajack_path).map_err(|e| {
        WjDirectiveError::Io(std::io::Error::new(
            e.kind(),
            format!(
                "Failed to open .wabbajack file {}: {}",
                wabbajack_path.display(),
                e
            ),
        ))
    })?;

    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        WjDirectiveError::ZipError(format!(
            "Failed to open ZIP {}: {}",
            wabbajack_path.display(),
            e
        ))
    })?;

    let mut entry = archive.by_name(entry_name).map_err(|e| {
        WjDirectiveError::FileNotFound(format!(
            "Entry '{}' not found in {}: {}",
            entry_name,
            wabbajack_path.display(),
            e
        ))
    })?;

    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Create parent directories for a path if they don't exist.
pub fn ensure_parent_dir(path: &Path) -> Result<(), WjDirectiveError> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

/// Verify that a file's xxHash64 matches the expected WjHash.
pub fn verify_output_hash(path: &Path, expected: &WjHash) -> Result<(), WjDirectiveError> {
    let actual = xxhash64_file(path).map_err(|e| {
        WjDirectiveError::Io(std::io::Error::new(
            e.kind(),
            format!("Failed to hash {}: {}", path.display(), e),
        ))
    })?;

    if actual != *expected {
        return Err(WjDirectiveError::HashMismatch {
            path: path.display().to_string(),
            expected: expected.0.clone(),
            actual: actual.0,
        });
    }
    Ok(())
}

/// Case-insensitive file lookup within a directory.
///
/// Wabbajack paths come from Windows where paths are case-insensitive.
/// On case-sensitive filesystems (Linux), we need to search for the file
/// by matching path components case-insensitively.
fn case_insensitive_find(base_dir: &Path, rel_path: &Path) -> Option<PathBuf> {
    let components: Vec<String> = rel_path
        .components()
        .filter_map(|c| {
            if let std::path::Component::Normal(s) = c {
                Some(s.to_string_lossy().to_lowercase())
            } else {
                None
            }
        })
        .collect();

    let mut current = base_dir.to_path_buf();
    for target_component in &components {
        let entries = std::fs::read_dir(&current).ok()?;
        let mut found = false;
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name == *target_component {
                current = entry.path();
                found = true;
                break;
            }
        }
        if !found {
            return None;
        }
    }

    if current.exists() && current != base_dir {
        Some(current)
    } else {
        None
    }
}

/// Parse a legacy pipe-delimited ArchiveHashPath string.
///
/// Wabbajack sometimes encodes archive hash paths as pipe-separated
/// strings like `"hash|relative\\path\\to\\file"` or nested:
/// `"hash|path|nested_hash|nested_path"`.
///
/// This extracts the first hash and first path components.
pub fn parse_archive_hash_path_string(s: &str) -> Option<(String, String)> {
    // Try pipe separator first, then colon
    let sep = if s.contains('|') {
        '|'
    } else if s.contains(':') {
        ':'
    } else {
        return None;
    };

    let parts: Vec<&str> = s.splitn(3, sep).collect();
    if parts.len() >= 2 {
        Some((parts[0].to_string(), normalize_wj_path(parts[1])))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_resolve_output_path_normalizes_backslashes() {
        let processor = DirectiveProcessor::new(
            PathBuf::from("/tmp/test.wabbajack"),
            HashMap::new(),
            PathBuf::from("/tmp/output"),
            PathBuf::from("/tmp/game"),
        );

        let result = processor.resolve_output_path(r"mods\SkyUI\SkyUI.esp").unwrap();
        assert_eq!(result, PathBuf::from("/tmp/output/mods/SkyUI/SkyUI.esp"));
    }

    #[test]
    fn test_resolve_output_path_forward_slashes() {
        let processor = DirectiveProcessor::new(
            PathBuf::from("/tmp/test.wabbajack"),
            HashMap::new(),
            PathBuf::from("/tmp/output"),
            PathBuf::from("/tmp/game"),
        );

        let result = processor.resolve_output_path("mods/SkyUI/SkyUI.esp").unwrap();
        assert_eq!(result, PathBuf::from("/tmp/output/mods/SkyUI/SkyUI.esp"));
    }

    #[test]
    fn test_parse_archive_hash_path_pipe() {
        let (hash, path) =
            parse_archive_hash_path_string("abc123==|textures\\armor\\cuirass.dds").unwrap();
        assert_eq!(hash, "abc123==");
        assert_eq!(path, "textures/armor/cuirass.dds");
    }

    #[test]
    fn test_parse_archive_hash_path_colon() {
        let (hash, path) =
            parse_archive_hash_path_string("abc123==:textures\\armor\\cuirass.dds").unwrap();
        assert_eq!(hash, "abc123==");
        assert_eq!(path, "textures/armor/cuirass.dds");
    }

    #[test]
    fn test_parse_archive_hash_path_no_separator() {
        assert!(parse_archive_hash_path_string("abc123==textures").is_none());
    }

    #[test]
    fn test_verify_output_hash_success() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.bin");
        let data = b"hello world";
        std::fs::write(&file_path, data).unwrap();

        let expected = xxhash64_bytes(data);
        assert!(verify_output_hash(&file_path, &expected).is_ok());
    }

    #[test]
    fn test_verify_output_hash_mismatch() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.bin");
        std::fs::write(&file_path, b"hello world").unwrap();

        let wrong_hash = WjHash::from_u64(0xDEADBEEF);
        let result = verify_output_hash(&file_path, &wrong_hash);
        assert!(result.is_err());
        match result.unwrap_err() {
            WjDirectiveError::HashMismatch { path, expected, .. } => {
                assert!(path.contains("test.bin"));
                assert_eq!(expected, wrong_hash.0);
            }
            other => panic!("Expected HashMismatch, got: {:?}", other),
        }
    }

    #[test]
    fn test_ensure_parent_dir_creates_parents() {
        let dir = TempDir::new().unwrap();
        let deep_path = dir.path().join("a").join("b").join("c").join("file.txt");
        assert!(!deep_path.parent().unwrap().exists());

        ensure_parent_dir(&deep_path).unwrap();
        assert!(deep_path.parent().unwrap().exists());
    }

    #[test]
    fn test_inline_file_from_zip() {
        let dir = TempDir::new().unwrap();
        let zip_path = dir.path().join("test.wabbajack");
        let output_dir = dir.path().join("output");

        // Create a test ZIP with an inline entry
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip_writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip_writer.start_file("42", options).unwrap();
            zip_writer.write_all(b"inline content here").unwrap();
            zip_writer.finish().unwrap();
        }

        let processor = DirectiveProcessor::new(
            zip_path,
            HashMap::new(),
            output_dir.clone(),
            PathBuf::from("/tmp/game"),
        );

        processor
            .process_inline_file("config/settings.ini", 42)
            .unwrap();

        let written = std::fs::read_to_string(output_dir.join("config/settings.ini")).unwrap();
        assert_eq!(written, "inline content here");
    }

    #[test]
    fn test_remapped_inline_file_substitution() {
        let dir = TempDir::new().unwrap();
        let zip_path = dir.path().join("test.wabbajack");
        let output_dir = dir.path().join("output");
        let game_dir = PathBuf::from("/bottles/skyrim/drive_c/Games/SkyrimSE");

        // Create a test ZIP with a remapped inline entry
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip_writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip_writer.start_file("99", options).unwrap();
            zip_writer
                .write_all(b"GamePath=GAMEDIR\\Data\nModPath=MO2DIR\\mods")
                .unwrap();
            zip_writer.finish().unwrap();
        }

        let processor = DirectiveProcessor::new(
            zip_path,
            HashMap::new(),
            output_dir.clone(),
            game_dir.clone(),
        );

        processor
            .process_remapped_inline_file("config/paths.ini", 99)
            .unwrap();

        let written = std::fs::read_to_string(output_dir.join("config/paths.ini")).unwrap();
        assert!(written.contains("/bottles/skyrim/drive_c/Games/SkyrimSE"));
        assert!(written.contains(&output_dir.to_string_lossy().to_string()));
    }

    #[test]
    fn test_from_archive_copies_file() {
        let dir = TempDir::new().unwrap();
        let archive_dir = dir.path().join("archives").join("abc123==");
        let output_dir = dir.path().join("output");

        // Set up an extracted archive directory
        let source_subdir = archive_dir.join("textures").join("armor");
        std::fs::create_dir_all(&source_subdir).unwrap();
        std::fs::write(source_subdir.join("cuirass.dds"), b"DDS texture data").unwrap();

        let mut archive_dirs = HashMap::new();
        archive_dirs.insert("abc123==".to_string(), archive_dir);

        let processor = DirectiveProcessor::new(
            PathBuf::from("/tmp/test.wabbajack"),
            archive_dirs,
            output_dir.clone(),
            PathBuf::from("/tmp/game"),
        );

        let ahp = ArchiveHashPath {
            base_hash: WjHash("abc123==".to_string()),
            parts: vec![
                "textures".to_string(),
                "armor".to_string(),
                "cuirass.dds".to_string(),
            ],
        };

        processor
            .process_from_archive("mods/ArmorMod/textures/armor/cuirass.dds", &ahp)
            .unwrap();

        let written =
            std::fs::read(output_dir.join("mods/ArmorMod/textures/armor/cuirass.dds")).unwrap();
        assert_eq!(written, b"DDS texture data");
    }

    #[test]
    fn test_case_insensitive_find() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("Textures").join("Armor");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("Cuirass.dds"), b"data").unwrap();

        // Search with different case
        let rel = PathBuf::from("textures/armor/cuirass.dds");
        let found = case_insensitive_find(dir.path(), &rel);
        assert!(found.is_some());
        assert!(found.unwrap().exists());
    }

    #[test]
    fn test_process_ignored_is_noop() {
        let processor = DirectiveProcessor::new(
            PathBuf::from("/tmp/test.wabbajack"),
            HashMap::new(),
            PathBuf::from("/tmp/output"),
            PathBuf::from("/tmp/game"),
        );

        // Should not error
        processor.process_ignored("some/file.txt", "Not needed");
    }

    #[test]
    fn test_directive_result_serializes() {
        let result = WjDirectiveResult {
            total_processed: 100,
            total_skipped: 5,
            warnings: vec!["warn1".to_string()],
            errors: vec![],
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"total_processed\":100"));
        assert!(json.contains("\"total_skipped\":5"));
    }

    #[test]
    fn test_read_wj_zip_entry_missing() {
        let dir = TempDir::new().unwrap();
        let zip_path = dir.path().join("test.wabbajack");

        // Create an empty ZIP
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let zip_writer = zip::ZipWriter::new(file);
            zip_writer.finish().unwrap();
        }

        let result = read_wj_zip_entry(&zip_path, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_dxgi_to_image_format_known_values() {
        use image_dds::ImageFormat;

        assert!(matches!(
            dxgi_to_image_format(71),
            ImageFormat::BC1RgbaUnorm
        ));
        assert!(matches!(
            dxgi_to_image_format(77),
            ImageFormat::BC3RgbaUnorm
        ));
        assert!(matches!(dxgi_to_image_format(80), ImageFormat::BC4RUnorm));
        assert!(matches!(dxgi_to_image_format(83), ImageFormat::BC5RgUnorm));
        assert!(matches!(dxgi_to_image_format(87), ImageFormat::Bgra8Unorm));
        assert!(matches!(
            dxgi_to_image_format(98),
            ImageFormat::BC7RgbaUnorm
        ));
        assert!(matches!(dxgi_to_image_format(28), ImageFormat::Rgba8Unorm));
    }

    #[test]
    fn test_dxgi_to_image_format_unknown_defaults_to_bc7() {
        use image_dds::ImageFormat;
        // Unknown format should default to BC7
        assert!(matches!(
            dxgi_to_image_format(999),
            ImageFormat::BC7RgbaUnorm
        ));
    }

    #[test]
    fn test_create_bsa_empty_file_states() {
        let dir = TempDir::new().unwrap();
        let output_dir = dir.path().join("output");
        std::fs::create_dir_all(&output_dir).unwrap();

        let processor = DirectiveProcessor::new(
            PathBuf::from("/tmp/test.wabbajack"),
            HashMap::new(),
            output_dir,
            PathBuf::from("/tmp/game"),
        );

        // Should succeed with no files (early return)
        let result = processor.process_create_bsa("test.bsa", 1, None, &[]);
        assert!(result.is_ok());
    }
}
