//! Minimal LSPK (Larian pak) reader for BG3.
//!
//! We do NOT implement a full pak extractor. The only goal is to locate
//! `meta.lsx` inside a `.pak` file, decompress it, and hand the bytes to
//! [`crate::bg3_lsx::read_meta_lsx_from_bytes`].
//!
//! # Format
//!
//! LSPK V16/V18 (V15 accepted, not deeply tested):
//!
//! ```text
//! [4 bytes]  magic: b"LSPK"
//! [4 bytes]  version: u32 LE  (15, 16, or 18 for BG3)
//! [8 bytes]  file_list_offset: u64 LE
//! [4 bytes]  file_list_size: u32 LE   (compressed size of file table block)
//! [1 byte]   flags
//! [1 byte]   priority
//! [16 bytes] md5
//! (V18 adds: [2 bytes] num_parts: u16 LE)
//! ```
//!
//! File table block (at `file_list_offset`):
//! ```text
//! [4 bytes] num_files: u32 LE
//! [4 bytes] compressed_size: u32 LE
//! [N bytes] LZ4-compressed FileEntry array
//! ```
//!
//! Each FileEntry (V18, 288 bytes):
//! ```text
//! [256 bytes] name: null-padded UTF-8
//! [4 bytes]   archive_part: u32 LE
//! [4 bytes]   flags: u32 LE  (lower 4 bits = compression: 0=None,1=Zlib,2=LZ4,3=LZ4HC,4=Zstd)
//! [8 bytes]   offset_in_file: u64 LE
//! [8 bytes]   size_on_disk: u64 LE  (compressed size)
//! [8 bytes]   uncompressed_size: u64 LE
//! ```
//!
//! # References
//! - LSLib / BGMM source: `LSLib/LS/PackageFile.cs`
//! - BG3 community documentation

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

// ─── Error ────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum PakError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid LSPK header: {0}")]
    InvalidHeader(String),
    #[error("unsupported LSPK version: {0}")]
    UnsupportedVersion(u32),
    #[error("unsupported compression algorithm: {0}")]
    UnsupportedCompression(u8),
    #[error("file not found in pak: {0}")]
    FileNotFound(String),
    #[error("decompression failed: {0}")]
    DecompressionFailed(String),
    #[error("lsx parse failed: {0}")]
    LsxParse(String),
}

// ─── Internal types ───────────────────────────────────────────────────────────

const LSPK_MAGIC: &[u8; 4] = b"LSPK";
/// Size of a single FileEntry on disk (V18). V16 uses the same layout.
const FILE_ENTRY_SIZE: usize = 288;

#[derive(Debug)]
struct PakHeader {
    /// LSPK format version (15, 16, or 18 for BG3). Retained for diagnostics.
    #[allow(dead_code)]
    version: u32,
    file_list_offset: u64,
    /// Total byte size of the file table block (header + compressed data).
    /// Retained for diagnostics and future multi-part pak support.
    #[allow(dead_code)]
    file_list_size: u32,
}

#[derive(Debug, Clone)]
struct PakEntry {
    name: String,
    /// Raw flags word; lower 4 bits are the compression algorithm.
    flags: u32,
    offset: u64,
    size_on_disk: u64,
    uncompressed_size: u64,
}

// ─── Low-level parsing ────────────────────────────────────────────────────────

fn read_u32_le(file: &mut File) -> Result<u32, PakError> {
    let mut buf = [0u8; 4];
    file.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64_le(file: &mut File) -> Result<u64, PakError> {
    let mut buf = [0u8; 8];
    file.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

fn read_header(file: &mut File) -> Result<PakHeader, PakError> {
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != LSPK_MAGIC {
        return Err(PakError::InvalidHeader(format!(
            "magic mismatch: got {:02x?}, expected {:02x?}",
            magic, LSPK_MAGIC
        )));
    }

    let version = read_u32_le(file)?;
    if !matches!(version, 15 | 16 | 18) {
        return Err(PakError::UnsupportedVersion(version));
    }

    let file_list_offset = read_u64_le(file)?;
    let file_list_size = read_u32_le(file)?;

    // Skip: flags (1), priority (1), md5 (16) = 18 bytes
    file.seek(SeekFrom::Current(18))?;

    // V18 adds num_parts: u16 — consume it so the cursor is correct.
    // V15/V16 don't have this field; we skip it unconditionally here because
    // we don't rely on num_parts (single-part is the overwhelmingly common case
    // and split-pak support is out of scope).
    if version == 18 {
        file.seek(SeekFrom::Current(2))?;
    }

    Ok(PakHeader {
        version,
        file_list_offset,
        file_list_size,
    })
}

/// Decompress the file table and return a `Vec<PakEntry>`.
///
/// The file table is always LZ4-compressed in V16+ paks.
fn read_entries(file: &mut File, header: &PakHeader) -> Result<Vec<PakEntry>, PakError> {
    file.seek(SeekFrom::Start(header.file_list_offset))?;

    let num_files = read_u32_le(file)? as usize;
    let compressed_size = read_u32_le(file)? as usize;

    let mut compressed = vec![0u8; compressed_size];
    file.read_exact(&mut compressed)?;

    // File table is always LZ4 (frame-less block format).
    let decompressed_size = num_files * FILE_ENTRY_SIZE;
    let decompressed = lz4_flex::block::decompress(&compressed, decompressed_size)
        .map_err(|e| PakError::DecompressionFailed(format!("file table lz4: {}", e)))?;

    let mut entries = Vec::with_capacity(num_files);
    for i in 0..num_files {
        let off = i * FILE_ENTRY_SIZE;
        let chunk = &decompressed[off..off + FILE_ENTRY_SIZE];

        let name = parse_null_padded(&chunk[0..256]);
        // chunk[256..260] = archive_part (u32) — skipped; we only support part 0
        let flags = u32::from_le_bytes(chunk[260..264].try_into().unwrap());
        let offset = u64::from_le_bytes(chunk[264..272].try_into().unwrap());
        let size_on_disk = u64::from_le_bytes(chunk[272..280].try_into().unwrap());
        let uncompressed_size = u64::from_le_bytes(chunk[280..288].try_into().unwrap());

        entries.push(PakEntry {
            name,
            flags,
            offset,
            size_on_disk,
            uncompressed_size,
        });
    }

    Ok(entries)
}

/// Parse a null-padded byte slice into a `String`.
///
/// Bytes after the first null are ignored. Non-UTF-8 bytes are replaced with
/// the Unicode replacement character (U+FFFD).
fn parse_null_padded(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// Read and decompress a single entry's data from the open pak file.
fn read_data(file: &mut File, entry: &PakEntry) -> Result<Vec<u8>, PakError> {
    file.seek(SeekFrom::Start(entry.offset))?;
    let mut compressed = vec![0u8; entry.size_on_disk as usize];
    file.read_exact(&mut compressed)?;

    let compression = (entry.flags & 0x0F) as u8;
    match compression {
        // No compression — data is stored verbatim.
        0 => Ok(compressed),

        // Zlib (deflate with header).
        1 => {
            let mut decoder = flate2::read::ZlibDecoder::new(&compressed[..]);
            let mut out = Vec::with_capacity(entry.uncompressed_size as usize);
            decoder
                .read_to_end(&mut out)
                .map_err(|e| PakError::DecompressionFailed(format!("zlib: {}", e)))?;
            Ok(out)
        }

        // LZ4 and LZ4HC share the same decompressor (HC is a compression-level hint only).
        2 | 3 => lz4_flex::block::decompress(&compressed, entry.uncompressed_size as usize)
            .map_err(|e| PakError::DecompressionFailed(format!("lz4: {}", e))),

        // Zstd.
        4 => zstd::decode_all(&compressed[..])
            .map_err(|e| PakError::DecompressionFailed(format!("zstd: {}", e))),

        other => Err(PakError::UnsupportedCompression(other)),
    }
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Read a named entry from a `.pak` and return its decompressed bytes.
///
/// Matching is case-insensitive and slash-direction-insensitive. If `entry_path`
/// is not an exact match, a suffix match is attempted so callers can pass just
/// `"meta.lsx"` or `"Mods/MyMod/meta.lsx"` interchangeably.
///
/// Returns [`PakError::FileNotFound`] if no matching entry is found.
pub fn read_pak_entry(pak_path: &Path, entry_path: &str) -> Result<Vec<u8>, PakError> {
    let mut file = File::open(pak_path)?;
    let header = read_header(&mut file)?;
    let entries = read_entries(&mut file, &header)?;

    let want = normalise_path(entry_path);
    let entry = find_entry(&entries, &want)
        .ok_or_else(|| PakError::FileNotFound(entry_path.to_string()))?
        .clone();

    read_data(&mut file, &entry)
}

/// Extract `meta.lsx` from a `.pak` and parse it into a [`crate::bg3_lsx::ModuleInfo`].
///
/// Locates the first entry whose normalised name ends with `meta.lsx` (case-insensitive),
/// decompresses it, and delegates to [`crate::bg3_lsx::read_meta_lsx_from_bytes`].
///
/// BG3 mods conventionally store meta.lsx at `Mods/<ModFolder>/meta.lsx` inside the pak.
pub fn read_pak_meta(pak_path: &Path) -> Result<crate::bg3_lsx::ModuleInfo, PakError> {
    let mut file = File::open(pak_path)?;
    let header = read_header(&mut file)?;
    let entries = read_entries(&mut file, &header)?;

    // Find any entry whose normalised name ends with "/meta.lsx" or IS "meta.lsx".
    let entry = entries
        .iter()
        .find(|e| {
            let n = normalise_path(&e.name);
            n == "meta.lsx" || n.ends_with("/meta.lsx")
        })
        .ok_or_else(|| PakError::FileNotFound("meta.lsx".to_string()))?
        .clone();

    let bytes = read_data(&mut file, &entry)?;

    crate::bg3_lsx::read_meta_lsx_from_bytes(&bytes)
        .map_err(|e| PakError::LsxParse(e.to_string()))
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Normalise a path for comparison: convert `\` → `/`, lowercase.
fn normalise_path(s: &str) -> String {
    s.replace('\\', "/").to_lowercase()
}

/// Find an entry by normalised exact match first, then suffix match.
fn find_entry<'a>(entries: &'a [PakEntry], want: &str) -> Option<&'a PakEntry> {
    entries
        .iter()
        .find(|e| normalise_path(&e.name) == want)
        .or_else(|| {
            entries
                .iter()
                .find(|e| normalise_path(&e.name).ends_with(&format!("/{}", want)))
        })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // ── Fixture meta.lsx (same XML used in bg3_lsx tests) ─────────────────────
    const META_LSX_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<save>
    <version major="4" minor="0" revision="0" build="0" />
    <region id="Config">
        <node id="root">
            <children>
                <node id="ModuleInfo">
                    <attribute id="Author" type="LSString" value="ModAuthor" />
                    <attribute id="Description" type="LSString" value="A test mod" />
                    <attribute id="Folder" type="LSString" value="TestMod" />
                    <attribute id="Name" type="LSString" value="Test Mod" />
                    <attribute id="UUID" type="FixedString" value="abcdef01-2345-6789-abcd-ef0123456789" />
                    <attribute id="Version64" type="int64" value="36028797018963968" />
                </node>
            </children>
        </node>
    </region>
</save>
"#;

    // ── Synthetic LSPK builder ─────────────────────────────────────────────────

    /// Build a minimal, valid V18 LSPK byte buffer containing a single
    /// uncompressed file at `entry_name` with `payload` as its content.
    ///
    /// Layout:
    ///   [header][payload bytes][file table block]
    ///
    /// The file table block is LZ4-compressed (as required by the format).
    fn make_minimal_lspk(entry_name: &str, payload: &[u8]) -> Vec<u8> {
        // --- 1. Build the file entry (288 bytes) --------------------------------
        let mut file_entry = [0u8; FILE_ENTRY_SIZE];

        // name field: null-padded UTF-8, 256 bytes
        let name_bytes = entry_name.as_bytes();
        let name_len = name_bytes.len().min(255);
        file_entry[..name_len].copy_from_slice(&name_bytes[..name_len]);
        // archive_part (chunk[256..260]) = 0 (already zeroed)
        // flags (chunk[260..264]) = 0 (no compression)
        // offset (chunk[264..272]) will be set below
        // size_on_disk (chunk[272..280]) = payload.len()
        let payload_len = payload.len() as u64;
        file_entry[272..280].copy_from_slice(&payload_len.to_le_bytes());
        // uncompressed_size (chunk[280..288]) = payload.len()
        file_entry[280..288].copy_from_slice(&payload_len.to_le_bytes());

        // --- 2. Compute header size so we know where the payload sits ----------
        //
        // V18 header layout:
        //   4  magic
        //   4  version
        //   8  file_list_offset
        //   4  file_list_size
        //   1  flags
        //   1  priority
        //  16  md5
        //   2  num_parts
        // = 40 bytes total
        let header_size: u64 = 40;

        // payload lives immediately after the header
        let payload_offset = header_size;

        // patch offset into the file entry
        file_entry[264..272].copy_from_slice(&payload_offset.to_le_bytes());

        // --- 3. LZ4-compress the file entry array ------------------------------
        // The pak format uses raw LZ4 block format (no prepended size header).
        let compressed_entries = lz4_flex::block::compress(&file_entry);

        // --- 4. File table block -----------------------------------------------
        let mut file_table: Vec<u8> = Vec::new();
        // num_files: u32 LE
        file_table.extend_from_slice(&1u32.to_le_bytes());
        // compressed_size: u32 LE
        file_table.extend_from_slice(&(compressed_entries.len() as u32).to_le_bytes());
        // compressed data
        file_table.extend_from_slice(&compressed_entries);

        // File table sits immediately after [header + payload].
        // payload_offset == header_size, so:  file_table_offset = header_size + payload_len
        let file_table_offset: u64 = header_size + payload_len;

        // --- 5. Assemble the full buffer ----------------------------------------
        let mut buf: Vec<u8> = Vec::new();

        // Header
        buf.extend_from_slice(LSPK_MAGIC);                                  // magic
        buf.extend_from_slice(&18u32.to_le_bytes());                         // version
        buf.extend_from_slice(&file_table_offset.to_le_bytes());             // file_list_offset
        buf.extend_from_slice(&(file_table.len() as u32).to_le_bytes());     // file_list_size
        buf.push(0u8);                                                        // flags
        buf.push(0u8);                                                        // priority
        buf.extend_from_slice(&[0u8; 16]);                                   // md5 (zeroed)
        buf.extend_from_slice(&1u16.to_le_bytes());                          // num_parts

        // Payload
        buf.extend_from_slice(payload);

        // File table
        buf.extend_from_slice(&file_table);

        buf
    }

    /// Write bytes to a temp file and return the NamedTempFile.
    fn write_temp(data: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(data).unwrap();
        f.flush().unwrap();
        f
    }

    // ── Error-path tests (no lz4 needed in payload) ────────────────────────────

    #[test]
    fn read_pak_entry_returns_invalid_header_for_garbage_magic() {
        let data = b"NOPE_NOT_A_PAK_FILE_AT_ALL_____";
        let f = write_temp(data);
        let err = read_pak_entry(f.path(), "meta.lsx").unwrap_err();
        assert!(
            matches!(err, PakError::InvalidHeader(_)),
            "expected InvalidHeader, got {:?}",
            err
        );
    }

    #[test]
    fn read_pak_entry_returns_unsupported_version_for_v14() {
        // Write LSPK magic + version 14 (unsupported).
        let mut data = Vec::new();
        data.extend_from_slice(LSPK_MAGIC);
        data.extend_from_slice(&14u32.to_le_bytes());
        // Pad to avoid IO error before the version check returns.
        data.extend_from_slice(&[0u8; 64]);
        let f = write_temp(&data);
        let err = read_pak_entry(f.path(), "meta.lsx").unwrap_err();
        assert!(
            matches!(err, PakError::UnsupportedVersion(14)),
            "expected UnsupportedVersion(14), got {:?}",
            err
        );
    }

    #[test]
    fn read_pak_meta_returns_invalid_header_for_garbage_magic() {
        let data = b"garbage_data_for_meta_test_____";
        let f = write_temp(data);
        let err = read_pak_meta(f.path()).unwrap_err();
        assert!(
            matches!(err, PakError::InvalidHeader(_)),
            "expected InvalidHeader, got {:?}",
            err
        );
    }

    // ── parse_null_padded unit tests ──────────────────────────────────────────

    #[test]
    fn parse_null_padded_stops_at_first_null() {
        let buf = b"hello\0world\0";
        assert_eq!(parse_null_padded(buf), "hello");
    }

    #[test]
    fn parse_null_padded_handles_full_buffer_no_null() {
        let buf = b"abcdef";
        assert_eq!(parse_null_padded(buf), "abcdef");
    }

    #[test]
    fn parse_null_padded_handles_empty_buffer() {
        assert_eq!(parse_null_padded(b""), "");
    }

    #[test]
    fn parse_null_padded_handles_null_at_start() {
        let buf = b"\0hello";
        assert_eq!(parse_null_padded(buf), "");
    }

    // ── normalise_path tests ──────────────────────────────────────────────────

    #[test]
    fn normalise_path_converts_backslash_to_forward_slash() {
        assert_eq!(normalise_path("Mods\\MyMod\\meta.lsx"), "mods/mymod/meta.lsx");
    }

    #[test]
    fn normalise_path_lowercases() {
        assert_eq!(normalise_path("Mods/MyMod/Meta.LSX"), "mods/mymod/meta.lsx");
    }

    // ── Synthetic LSPK round-trip ─────────────────────────────────────────────

    #[test]
    fn read_pak_entry_extracts_uncompressed_file() {
        let payload = b"Hello, LSPK world!";
        let pak_bytes = make_minimal_lspk("test/hello.txt", payload);
        let f = write_temp(&pak_bytes);

        // Exact match
        let got = read_pak_entry(f.path(), "test/hello.txt").unwrap();
        assert_eq!(got, payload);
    }

    #[test]
    fn read_pak_entry_matches_case_insensitively() {
        let payload = b"case insensitive data";
        let pak_bytes = make_minimal_lspk("Mods/MyMod/meta.lsx", payload);
        let f = write_temp(&pak_bytes);

        // Lowercase variant of the path should still match.
        let got = read_pak_entry(f.path(), "mods/mymod/meta.lsx").unwrap();
        assert_eq!(got, payload);
    }

    #[test]
    fn read_pak_entry_returns_file_not_found_for_missing_entry() {
        let pak_bytes = make_minimal_lspk("test/hello.txt", b"data");
        let f = write_temp(&pak_bytes);

        let err = read_pak_entry(f.path(), "nonexistent.dat").unwrap_err();
        assert!(
            matches!(err, PakError::FileNotFound(_)),
            "expected FileNotFound, got {:?}",
            err
        );
    }

    #[test]
    fn read_pak_meta_extracts_module_info_from_uncompressed_pak() {
        // Build a pak containing meta.lsx at Mods/TestMod/meta.lsx.
        let pak_bytes = make_minimal_lspk("Mods/TestMod/meta.lsx", META_LSX_XML);
        let f = write_temp(&pak_bytes);

        let info = read_pak_meta(f.path()).unwrap();
        assert_eq!(info.folder, "TestMod");
        assert_eq!(info.name, "Test Mod");
        assert_eq!(info.uuid, "abcdef01-2345-6789-abcd-ef0123456789");
        assert_eq!(info.version64, "36028797018963968");
        assert_eq!(info.author.as_deref(), Some("ModAuthor"));
    }

    #[test]
    fn read_pak_meta_finds_meta_lsx_without_full_path() {
        // Store meta.lsx at the root (no directory prefix).
        let pak_bytes = make_minimal_lspk("meta.lsx", META_LSX_XML);
        let f = write_temp(&pak_bytes);

        let info = read_pak_meta(f.path()).unwrap();
        assert_eq!(info.folder, "TestMod");
    }

    #[test]
    fn read_pak_meta_returns_file_not_found_when_no_meta_lsx() {
        let pak_bytes = make_minimal_lspk("Mods/TestMod/other.lsx", b"not meta");
        let f = write_temp(&pak_bytes);

        let err = read_pak_meta(f.path()).unwrap_err();
        assert!(
            matches!(err, PakError::FileNotFound(_)),
            "expected FileNotFound, got {:?}",
            err
        );
    }
}
