use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use xxhash_rust::xxh64::{self, Xxh64};

// ---------------------------------------------------------------------------
// xxHash64 wrapper (base64-encoded, matching Wabbajack format)
// ---------------------------------------------------------------------------

/// Base64-encoded xxHash64 hash (e.g., `"eSIyd+KOG3s="`).
/// Wabbajack uses standard base64 with padding.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WjHash(pub String);

impl WjHash {
    pub fn from_u64(val: u64) -> Self {
        // Wabbajack (C#/.NET) uses BitConverter.GetBytes(long) which produces
        // little-endian bytes on x86/x64, then base64-encodes them.
        let bytes = val.to_le_bytes();
        WjHash(base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn to_u64(&self) -> Option<u64> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&self.0)
            .ok()?;
        if bytes.len() != 8 {
            return None;
        }
        Some(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for WjHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Compute xxHash64 of a byte slice and return as base64 WjHash.
pub fn xxhash64_bytes(data: &[u8]) -> WjHash {
    WjHash::from_u64(xxh64::xxh64(data, 0))
}

/// Compute xxHash64 of a file and return as base64 WjHash.
///
/// Streams the file through an `Xxh64` hasher in 1 MiB chunks so that hashing
/// a 10–40 GB Wabbajack archive does not require buffering the entire file in
/// memory — the previous implementation would OOM on 16 GB Steam Decks.
pub fn xxhash64_file(path: &Path) -> Result<WjHash, std::io::Error> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 1024 * 1024]; // 1 MiB streaming buffer
    let mut hasher = Xxh64::new(0);
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let hash_val = hasher.digest();
    Ok(WjHash::from_u64(hash_val))
}

// ---------------------------------------------------------------------------
// $type field deserializer — strips C# namespace suffix
// ---------------------------------------------------------------------------

/// Strips `", Wabbajack.DTOs"` etc. from the `$type` field.
/// E.g., `"FromArchive, Wabbajack.DTOs"` → `"FromArchive"`
pub fn strip_csharp_type(s: &str) -> &str {
    s.split(',').next().unwrap_or(s).trim()
}

// ---------------------------------------------------------------------------
// Archive download source states
// ---------------------------------------------------------------------------

/// All download source types from Wabbajack's `DownloadStates/` DTOs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "$type")]
pub enum WjArchiveState {
    #[serde(
        alias = "NexusDownloader+State, Wabbajack.Lib",
        alias = "NexusDownloader, Wabbajack.Lib",
        alias = "Nexus, Wabbajack.DTOs",
        alias = "NexusDownloader+State"
    )]
    Nexus {
        #[serde(rename = "Game", alias = "GameName", default)]
        game: String,
        #[serde(rename = "ModID", alias = "modID", default)]
        mod_id: i64,
        #[serde(rename = "FileID", alias = "fileID", default)]
        file_id: i64,
    },

    #[serde(
        alias = "HttpDownloader+State, Wabbajack.Lib",
        alias = "HttpDownloader, Wabbajack.Lib",
        alias = "Http, Wabbajack.DTOs",
        alias = "HttpDownloader+State"
    )]
    Http {
        #[serde(rename = "Url", alias = "url", default)]
        url: String,
        #[serde(rename = "Headers", alias = "headers", default)]
        headers: Vec<String>,
    },

    #[serde(
        alias = "GoogleDriveDownloader+State, Wabbajack.Lib",
        alias = "GoogleDriveDownloader, Wabbajack.Lib",
        alias = "GoogleDrive, Wabbajack.DTOs",
        alias = "GoogleDriveDownloader+State"
    )]
    GoogleDrive {
        #[serde(rename = "Id", alias = "id", default)]
        id: String,
    },

    #[serde(
        alias = "MegaDownloader+State, Wabbajack.Lib",
        alias = "MegaDownloader, Wabbajack.Lib",
        alias = "Mega, Wabbajack.DTOs",
        alias = "MegaDownloader+State"
    )]
    Mega {
        #[serde(rename = "Url", alias = "url", default)]
        url: String,
    },

    #[serde(
        alias = "MediaFireDownloader+State, Wabbajack.Lib",
        alias = "MediaFireDownloader, Wabbajack.Lib",
        alias = "MediaFire, Wabbajack.DTOs",
        alias = "MediaFireDownloader+State"
    )]
    MediaFire {
        #[serde(rename = "Url", alias = "url", default)]
        url: String,
    },

    #[serde(
        alias = "ModDBDownloader+State, Wabbajack.Lib",
        alias = "ModDBDownloader, Wabbajack.Lib",
        alias = "ModDB, Wabbajack.DTOs",
        alias = "ModDBDownloader+State"
    )]
    ModDB {
        #[serde(rename = "Url", alias = "url", default)]
        url: String,
    },

    #[serde(
        alias = "WabbajackCDNDownloader+State, Wabbajack.Lib",
        alias = "WabbajackCDNDownloader, Wabbajack.Lib",
        alias = "WabbajackCDN, Wabbajack.DTOs",
        alias = "WabbajackCDNDownloader+State"
    )]
    WabbajackCDN {
        #[serde(rename = "Url", alias = "url", default)]
        url: String,
    },

    #[serde(
        alias = "GameFileSourceDownloader+State, Wabbajack.Lib",
        alias = "GameFileSourceDownloader, Wabbajack.Lib",
        alias = "GameFileSource, Wabbajack.DTOs",
        alias = "GameFileSourceDownloader+State"
    )]
    GameFileSource {
        #[serde(rename = "Game", alias = "game", default)]
        game: String,
        #[serde(rename = "GameFile", alias = "gameFile", default)]
        game_file: String,
        #[serde(rename = "GameVersion", alias = "gameVersion", default)]
        game_version: String,
        #[serde(rename = "Hash", alias = "hash", default)]
        hash: WjHash,
    },

    #[serde(
        alias = "ManualDownloader+State, Wabbajack.Lib",
        alias = "ManualDownloader, Wabbajack.Lib",
        alias = "Manual, Wabbajack.DTOs",
        alias = "ManualDownloader+State"
    )]
    Manual {
        #[serde(rename = "Url", alias = "url", default)]
        url: String,
        #[serde(rename = "Prompt", alias = "prompt", default)]
        prompt: String,
    },

    #[serde(
        alias = "LoversLabDownloader+State, Wabbajack.Lib",
        alias = "LoversLabDownloader, Wabbajack.Lib",
        alias = "LoversLab, Wabbajack.DTOs",
        alias = "LoversLabOAuthDownloader+State, Wabbajack.Lib",
        alias = "LoversLabOAuthDownloader, Wabbajack.Lib",
        alias = "LoversLabOAuthDownloader+State",
        alias = "LoversLabDownloader+State"
    )]
    LoversLab {
        #[serde(rename = "IPS4Url", alias = "url", default)]
        url: String,
        #[serde(rename = "IPS4Mod", alias = "ips4Mod", default)]
        ips4_mod: Option<i64>,
        #[serde(rename = "IPS4File", alias = "ips4File", default)]
        ips4_file: Option<String>,
    },

    #[serde(
        alias = "VectorPlexusDownloader+State, Wabbajack.Lib",
        alias = "VectorPlexusDownloader, Wabbajack.Lib",
        alias = "VectorPlexus, Wabbajack.DTOs",
        alias = "VectorPlexusOAuthDownloader+State, Wabbajack.Lib",
        alias = "VectorPlexusOAuthDownloader, Wabbajack.Lib",
        alias = "VectorPlexusOAuthDownloader+State",
        alias = "VectorPlexusDownloader+State"
    )]
    VectorPlexus {
        #[serde(rename = "IPS4Url", alias = "url", default)]
        url: String,
        #[serde(rename = "IPS4Mod", alias = "ips4Mod", default)]
        ips4_mod: Option<i64>,
        #[serde(rename = "IPS4File", alias = "ips4File", default)]
        ips4_file: Option<String>,
    },

    #[serde(
        alias = "TESAllianceDownloader+State, Wabbajack.Lib",
        alias = "TESAllianceDownloader, Wabbajack.Lib",
        alias = "TESAlliance, Wabbajack.DTOs",
        alias = "TESAllianceDownloader+State"
    )]
    TESAlliance {
        #[serde(rename = "IPS4Url", alias = "url", default)]
        url: String,
        #[serde(rename = "IPS4Mod", alias = "ips4Mod", default)]
        ips4_mod: Option<i64>,
        #[serde(rename = "IPS4File", alias = "ips4File", default)]
        ips4_file: Option<String>,
    },

    #[serde(
        alias = "BethesdaNetDownloader+State, Wabbajack.Lib",
        alias = "BethesdaNetDownloader, Wabbajack.Lib",
        alias = "Bethesda, Wabbajack.DTOs",
        alias = "BethesdaNet, Wabbajack.DTOs",
        alias = "BethesdaNetDownloader+State"
    )]
    Bethesda {
        #[serde(flatten)]
        extra: serde_json::Value,
    },
}

impl WjArchiveState {
    /// Human-readable source type name for display.
    pub fn source_type_name(&self) -> &str {
        match self {
            WjArchiveState::Nexus { .. } => "Nexus",
            WjArchiveState::Http { .. } => "HTTP",
            WjArchiveState::GoogleDrive { .. } => "GoogleDrive",
            WjArchiveState::Mega { .. } => "Mega",
            WjArchiveState::MediaFire { .. } => "MediaFire",
            WjArchiveState::ModDB { .. } => "ModDB",
            WjArchiveState::WabbajackCDN { .. } => "WabbajackCDN",
            WjArchiveState::GameFileSource { .. } => "GameFile",
            WjArchiveState::Manual { .. } => "Manual",
            WjArchiveState::LoversLab { .. } => "LoversLab",
            WjArchiveState::VectorPlexus { .. } => "VectorPlexus",
            WjArchiveState::TESAlliance { .. } => "TESAlliance",
            WjArchiveState::Bethesda { .. } => "Bethesda",
        }
    }
}

// ---------------------------------------------------------------------------
// Directive types
// ---------------------------------------------------------------------------

/// Image/texture state for TransformedTexture directives.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ImageState {
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// DXGI format — either a numeric ID (old WJ) or string name like "BC7_UNORM" (new WJ).
    #[serde(default, deserialize_with = "deserialize_dxgi_format")]
    pub format: u32,
    #[serde(default)]
    pub perceptual_hash: Option<String>,
    #[serde(default)]
    pub mip_levels: u32,
}

/// Deserialize DXGI format as either a u32 or a string name.
fn deserialize_dxgi_format<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct DxgiVisitor;

    impl<'de> de::Visitor<'de> for DxgiVisitor {
        type Value = u32;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a DXGI format number or name string")
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<u32, E> {
            Ok(v as u32)
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<u32, E> {
            Ok(v as u32)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<u32, E> {
            Ok(dxgi_format_from_name(v))
        }

        fn visit_string<E: de::Error>(self, v: String) -> Result<u32, E> {
            Ok(dxgi_format_from_name(&v))
        }

        fn visit_none<E: de::Error>(self) -> Result<u32, E> {
            Ok(0)
        }

        fn visit_unit<E: de::Error>(self) -> Result<u32, E> {
            Ok(0)
        }
    }

    deserializer.deserialize_any(DxgiVisitor)
}

/// Map DXGI format name strings to their numeric values.
fn dxgi_format_from_name(name: &str) -> u32 {
    match name {
        "R8G8B8A8_UNORM" => 28,
        "R8G8B8A8_UNORM_SRGB" => 29,
        "B8G8R8A8_UNORM" => 87,
        "B8G8R8A8_UNORM_SRGB" => 91,
        "BC1_UNORM" => 71,
        "BC1_UNORM_SRGB" => 72,
        "BC2_UNORM" => 74,
        "BC2_UNORM_SRGB" => 75,
        "BC3_UNORM" => 77,
        "BC3_UNORM_SRGB" => 78,
        "BC4_UNORM" => 80,
        "BC4_SNORM" => 81,
        "BC5_UNORM" => 83,
        "BC5_SNORM" => 84,
        "BC6H_UF16" => 95,
        "BC6H_SF16" => 96,
        "BC7_UNORM" => 98,
        "BC7_UNORM_SRGB" => 99,
        "R8_UNORM" => 61,
        "R16_FLOAT" => 54,
        "R32_FLOAT" => 41,
        "R16G16_FLOAT" => 34,
        "R16G16B16A16_FLOAT" => 10,
        "R32G32B32A32_FLOAT" => 2,
        "A8_UNORM" => 65,
        _ => {
            log::warn!("Unknown DXGI format name '{}', defaulting to DXGI_FORMAT_UNKNOWN (0). DDS transforms using this format may fail.", name);
            0
        }
    }
}

/// Source reference for MergedPatch directives.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SourcePatch {
    pub hash: WjHash,
    pub relative_path: String,
}

/// BSA file state for CreateBSA directives.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BsaState {
    #[serde(default, rename = "$type")]
    pub state_type: String,
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

/// BSA file entry state for CreateBSA directives.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BsaFileState {
    pub path: String,
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

/// All directive types from Wabbajack's `Directives/` DTOs.
/// Uses internally tagged representation via `$type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "$type")]
pub enum WjDirective {
    #[serde(
        alias = "FromArchive, Wabbajack.DTOs",
        alias = "FromArchive, Wabbajack.Lib"
    )]
    FromArchive {
        #[serde(rename = "Hash", default)]
        hash: WjHash,
        #[serde(rename = "Size", default)]
        size: i64,
        #[serde(rename = "To", default)]
        to: String,
        #[serde(rename = "ArchiveHashPath", default)]
        archive_hash_path: ArchiveHashPath,
    },

    #[serde(
        alias = "PatchedFromArchive, Wabbajack.DTOs",
        alias = "PatchedFromArchive, Wabbajack.Lib"
    )]
    PatchedFromArchive {
        #[serde(rename = "Hash", default)]
        hash: WjHash,
        #[serde(rename = "Size", default)]
        size: i64,
        #[serde(rename = "To", default)]
        to: String,
        #[serde(rename = "ArchiveHashPath", default)]
        archive_hash_path: ArchiveHashPath,
        #[serde(rename = "FromHash", default)]
        from_hash: WjHash,
        #[serde(
            rename = "PatchID",
            default,
            deserialize_with = "deserialize_source_data_id"
        )]
        patch_id: String,
    },

    #[serde(
        alias = "InlineFile, Wabbajack.DTOs",
        alias = "InlineFile, Wabbajack.Lib"
    )]
    InlineFile {
        #[serde(rename = "Hash", default)]
        hash: WjHash,
        #[serde(rename = "Size", default)]
        size: i64,
        #[serde(rename = "To", default)]
        to: String,
        /// Entry name in the .wabbajack ZIP. Was i64 in old WJ, UUID string in newer WJ.
        #[serde(
            rename = "SourceDataID",
            default,
            deserialize_with = "deserialize_source_data_id"
        )]
        source_data_id: String,
    },

    #[serde(
        alias = "RemappedInlineFile, Wabbajack.DTOs",
        alias = "RemappedInlineFile, Wabbajack.Lib"
    )]
    RemappedInlineFile {
        #[serde(rename = "Hash", default)]
        hash: WjHash,
        #[serde(rename = "Size", default)]
        size: i64,
        #[serde(rename = "To", default)]
        to: String,
        /// Entry name in the .wabbajack ZIP. Was i64 in old WJ, UUID string in newer WJ.
        #[serde(
            rename = "SourceDataID",
            default,
            deserialize_with = "deserialize_source_data_id"
        )]
        source_data_id: String,
    },

    #[serde(
        alias = "CreateBSA, Wabbajack.DTOs",
        alias = "CreateBSA, Wabbajack.Lib"
    )]
    CreateBSA {
        #[serde(rename = "Hash", default)]
        hash: WjHash,
        #[serde(rename = "Size", default)]
        size: i64,
        #[serde(rename = "To", default)]
        to: String,
        #[serde(
            rename = "TempID",
            default,
            deserialize_with = "deserialize_source_data_id"
        )]
        temp_id: String,
        #[serde(rename = "State", default)]
        state: Option<BsaState>,
        #[serde(rename = "FileStates", default)]
        file_states: Vec<BsaFileState>,
    },

    #[serde(
        alias = "TransformedTexture, Wabbajack.DTOs",
        alias = "TransformedTexture, Wabbajack.Lib"
    )]
    TransformedTexture {
        #[serde(rename = "Hash", default)]
        hash: WjHash,
        #[serde(rename = "Size", default)]
        size: i64,
        #[serde(rename = "To", default)]
        to: String,
        #[serde(rename = "ArchiveHashPath", default)]
        archive_hash_path: ArchiveHashPath,
        #[serde(rename = "ImageState", default)]
        image_state: Option<ImageState>,
    },

    #[serde(
        alias = "MergedPatch, Wabbajack.DTOs",
        alias = "MergedPatch, Wabbajack.Lib"
    )]
    MergedPatch {
        #[serde(rename = "Hash", default)]
        hash: WjHash,
        #[serde(rename = "Size", default)]
        size: i64,
        #[serde(rename = "To", default)]
        to: String,
        #[serde(
            rename = "PatchID",
            default,
            deserialize_with = "deserialize_source_data_id"
        )]
        patch_id: String,
        #[serde(rename = "Sources", default)]
        sources: Vec<SourcePatch>,
    },

    #[serde(
        alias = "IgnoredDirectly, Wabbajack.DTOs",
        alias = "IgnoredDirectly, Wabbajack.Lib"
    )]
    IgnoredDirectly {
        #[serde(rename = "Hash", default)]
        hash: WjHash,
        #[serde(rename = "Size", default)]
        size: i64,
        #[serde(rename = "To", default)]
        to: String,
        #[serde(rename = "Reason", default)]
        reason: String,
    },
}

impl WjDirective {
    /// Get the destination path for this directive.
    pub fn to_path(&self) -> &str {
        match self {
            WjDirective::FromArchive { to, .. } => to,
            WjDirective::PatchedFromArchive { to, .. } => to,
            WjDirective::InlineFile { to, .. } => to,
            WjDirective::RemappedInlineFile { to, .. } => to,
            WjDirective::CreateBSA { to, .. } => to,
            WjDirective::TransformedTexture { to, .. } => to,
            WjDirective::MergedPatch { to, .. } => to,
            WjDirective::IgnoredDirectly { to, .. } => to,
        }
    }

    /// Get the expected output hash.
    pub fn hash(&self) -> &WjHash {
        match self {
            WjDirective::FromArchive { hash, .. } => hash,
            WjDirective::PatchedFromArchive { hash, .. } => hash,
            WjDirective::InlineFile { hash, .. } => hash,
            WjDirective::RemappedInlineFile { hash, .. } => hash,
            WjDirective::CreateBSA { hash, .. } => hash,
            WjDirective::TransformedTexture { hash, .. } => hash,
            WjDirective::MergedPatch { hash, .. } => hash,
            WjDirective::IgnoredDirectly { hash, .. } => hash,
        }
    }

    /// Get the expected output size.
    pub fn size(&self) -> i64 {
        match self {
            WjDirective::FromArchive { size, .. } => *size,
            WjDirective::PatchedFromArchive { size, .. } => *size,
            WjDirective::InlineFile { size, .. } => *size,
            WjDirective::RemappedInlineFile { size, .. } => *size,
            WjDirective::CreateBSA { size, .. } => *size,
            WjDirective::TransformedTexture { size, .. } => *size,
            WjDirective::MergedPatch { size, .. } => *size,
            WjDirective::IgnoredDirectly { size, .. } => *size,
        }
    }

    /// Human-readable directive type name.
    pub fn kind_name(&self) -> &str {
        match self {
            WjDirective::FromArchive { .. } => "FromArchive",
            WjDirective::PatchedFromArchive { .. } => "PatchedFromArchive",
            WjDirective::InlineFile { .. } => "InlineFile",
            WjDirective::RemappedInlineFile { .. } => "RemappedInlineFile",
            WjDirective::CreateBSA { .. } => "CreateBSA",
            WjDirective::TransformedTexture { .. } => "TransformedTexture",
            WjDirective::MergedPatch { .. } => "MergedPatch",
            WjDirective::IgnoredDirectly { .. } => "IgnoredDirectly",
        }
    }
}

// ---------------------------------------------------------------------------
// ArchiveHashPath — references a file inside a downloaded archive
// ---------------------------------------------------------------------------

/// Wabbajack's ArchiveHashPath: identifies a file by the archive's hash
/// plus the relative path within the archive.
///
/// Formats:
/// - Old WJ object: `{"BaseHash": "abc=", "Parts": ["path", "to", "file"]}`
/// - New WJ array: `["abc=", "path\\to\\file"]` or `["abc=", "archive.bsa", "inner\\path"]`
/// - String: `"abc=|path\\to\\file"` (pipe-delimited)
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArchiveHashPath {
    pub base_hash: WjHash,
    pub parts: Vec<String>,
}

impl<'de> serde::Deserialize<'de> for ArchiveHashPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de;

        struct AhpVisitor;

        impl<'de> de::Visitor<'de> for AhpVisitor {
            type Value = ArchiveHashPath;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an ArchiveHashPath object, array, or string")
            }

            // New format: flat array ["hash", "path1", "path2", ...]
            fn visit_seq<A: de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<ArchiveHashPath, A::Error> {
                let hash: String = seq.next_element()?.unwrap_or_default();
                let mut parts = Vec::new();
                while let Some(part) = seq.next_element::<String>()? {
                    parts.push(part);
                }
                Ok(ArchiveHashPath {
                    base_hash: WjHash(hash),
                    parts,
                })
            }

            // Old format: object with BaseHash and Parts
            fn visit_map<M: de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<ArchiveHashPath, M::Error> {
                let mut base_hash = WjHash(String::new());
                let mut parts = Vec::new();

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "BaseHash" | "baseHash" | "base_hash" => {
                            base_hash = WjHash(map.next_value::<String>()?);
                        }
                        "Parts" | "parts" => {
                            // Parts can be a string or array
                            let val: serde_json::Value = map.next_value()?;
                            match val {
                                serde_json::Value::String(s) => {
                                    parts = s.split('\\').map(|s| s.to_string()).collect();
                                }
                                serde_json::Value::Array(arr) => {
                                    for v in arr {
                                        if let Some(s) = v.as_str() {
                                            parts.push(s.to_string());
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ => {
                            let _: serde_json::Value = map.next_value()?;
                        }
                    }
                }

                Ok(ArchiveHashPath { base_hash, parts })
            }

            // String format: "hash|path\\to\\file"
            fn visit_str<E: de::Error>(self, v: &str) -> Result<ArchiveHashPath, E> {
                if let Some((hash, path)) = v.split_once('|') {
                    Ok(ArchiveHashPath {
                        base_hash: WjHash(hash.to_string()),
                        parts: path.split('\\').map(|s| s.to_string()).collect(),
                    })
                } else {
                    Ok(ArchiveHashPath {
                        base_hash: WjHash(v.to_string()),
                        parts: Vec::new(),
                    })
                }
            }

            fn visit_none<E: de::Error>(self) -> Result<ArchiveHashPath, E> {
                Ok(ArchiveHashPath::default())
            }

            fn visit_unit<E: de::Error>(self) -> Result<ArchiveHashPath, E> {
                Ok(ArchiveHashPath::default())
            }
        }

        deserializer.deserialize_any(AhpVisitor)
    }
}

impl ArchiveHashPath {
    /// Get the relative path within the archive (joining all parts).
    pub fn relative_path(&self) -> PathBuf {
        let mut path = PathBuf::new();
        for part in &self.parts {
            path.push(part);
        }
        path
    }
}

/// Deserialize SourceDataID as either an integer (old WJ) or UUID string (new WJ).
fn deserialize_source_data_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct SourceDataIdVisitor;

    impl<'de> de::Visitor<'de> for SourceDataIdVisitor {
        type Value = String;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an integer or string")
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<String, E> {
            Ok(v.to_string())
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<String, E> {
            Ok(v.to_string())
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<String, E> {
            Ok(v.to_string())
        }

        fn visit_string<E: de::Error>(self, v: String) -> Result<String, E> {
            Ok(v)
        }

        fn visit_none<E: de::Error>(self) -> Result<String, E> {
            Ok(String::new())
        }

        fn visit_unit<E: de::Error>(self) -> Result<String, E> {
            Ok(String::new())
        }
    }

    deserializer.deserialize_any(SourceDataIdVisitor)
}

// ---------------------------------------------------------------------------
// Typed archive (replaces serde_json::Value state)
// ---------------------------------------------------------------------------

/// A typed archive entry for the modlist.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct WjTypedArchive {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub hash: WjHash,
    #[serde(default)]
    pub size: u64,
    pub state: WjArchiveState,
}

// ---------------------------------------------------------------------------
// Typed modlist (the full parsed structure)
// ---------------------------------------------------------------------------

/// Top-level modlist with strongly-typed directives and archive states.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct WjTypedModlist {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(
        default,
        rename = "GameType",
        deserialize_with = "crate::wabbajack::deserialize_game_type"
    )]
    pub game_type: u32,
    #[serde(default)]
    pub archives: Vec<WjTypedArchive>,
    #[serde(default)]
    pub directives: Vec<WjDirective>,
    #[serde(default, rename = "IsNSFW")]
    pub is_nsfw: bool,
    #[serde(default)]
    pub wabbajack_version: String,
}

impl WjTypedModlist {
    /// Extract the required game version from `GameFileSource` archives.
    /// Returns the most common `GameVersion` string found (e.g. "1.6.1170.0").
    /// Returns `None` if no `GameFileSource` archives exist or none have a version set.
    pub fn required_game_version(&self) -> Option<String> {
        let mut versions: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for archive in &self.archives {
            if let WjArchiveState::GameFileSource { game_version, .. } = &archive.state {
                let v = game_version.trim();
                if !v.is_empty() && v != "0.0.0.0" {
                    *versions.entry(v.to_string()).or_insert(0) += 1;
                }
            }
        }
        versions
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(v, _)| v)
    }
}

// ---------------------------------------------------------------------------
// Install status types
// ---------------------------------------------------------------------------

/// Installation status for a Wabbajack modlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WjInstallStatus {
    Pending,
    Preflight,
    Downloading,
    Extracting,
    Processing,
    Deploying,
    Completed,
    Failed,
    Cancelled,
}

impl WjInstallStatus {
    pub fn as_str(&self) -> &str {
        match self {
            WjInstallStatus::Pending => "pending",
            WjInstallStatus::Preflight => "preflight",
            WjInstallStatus::Downloading => "downloading",
            WjInstallStatus::Extracting => "extracting",
            WjInstallStatus::Processing => "processing",
            WjInstallStatus::Deploying => "deploying",
            WjInstallStatus::Completed => "completed",
            WjInstallStatus::Failed => "failed",
            WjInstallStatus::Cancelled => "cancelled",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "pending" => WjInstallStatus::Pending,
            "preflight" => WjInstallStatus::Preflight,
            "downloading" => WjInstallStatus::Downloading,
            "extracting" => WjInstallStatus::Extracting,
            "processing" => WjInstallStatus::Processing,
            "deploying" => WjInstallStatus::Deploying,
            "completed" => WjInstallStatus::Completed,
            "failed" => WjInstallStatus::Failed,
            "cancelled" => WjInstallStatus::Cancelled,
            _ => WjInstallStatus::Pending,
        }
    }
}

/// Per-archive download status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WjArchiveDownloadStatus {
    Pending,
    Downloading,
    Downloaded,
    Verified,
    Failed,
    Skipped,
}

impl WjArchiveDownloadStatus {
    pub fn as_str(&self) -> &str {
        match self {
            WjArchiveDownloadStatus::Pending => "pending",
            WjArchiveDownloadStatus::Downloading => "downloading",
            WjArchiveDownloadStatus::Downloaded => "downloaded",
            WjArchiveDownloadStatus::Verified => "verified",
            WjArchiveDownloadStatus::Failed => "failed",
            WjArchiveDownloadStatus::Skipped => "skipped",
        }
    }
}

// ---------------------------------------------------------------------------
// Pre-flight result
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WjPreflightResult {
    pub passed: bool,
    pub available_disk_space: u64,
    pub required_disk_space: u64,
    pub archive_count: usize,
    pub directive_count: usize,
    pub nexus_premium: bool,
    pub nexus_archives: usize,
    pub manual_archives: usize,
    pub game_detected: bool,
    pub issues: Vec<WjPreflightIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WjPreflightIssue {
    pub severity: String, // "error", "warning", "info"
    pub message: String,
}

// ---------------------------------------------------------------------------
// Path normalization (Windows → Unix)
// ---------------------------------------------------------------------------

/// Normalize a Wabbajack path (Windows backslashes) to Unix forward slashes.
pub fn normalize_wj_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// Apply GAMEDIR and MO2DIR substitutions to a path.
pub fn substitute_wj_path(path: &str, game_dir: &Path, install_dir: &Path) -> String {
    let normalized = normalize_wj_path(path);
    normalized
        .replace("GAMEDIR", &game_dir.to_string_lossy())
        .replace("MO2DIR", &install_dir.to_string_lossy())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wj_hash_roundtrip() {
        let hash = WjHash::from_u64(0x1234567890ABCDEF);
        assert_eq!(hash.to_u64(), Some(0x1234567890ABCDEF));
    }

    #[test]
    fn test_wj_hash_base64_format() {
        // Known value: Wabbajack uses standard base64 with padding
        let hash = WjHash::from_u64(0);
        assert_eq!(hash.0, "AAAAAAAAAAA=");
    }

    #[test]
    fn test_xxhash64_bytes() {
        let hash = xxhash64_bytes(b"hello world");
        assert!(!hash.is_empty());
        // Verify it produces a consistent hash
        let hash2 = xxhash64_bytes(b"hello world");
        assert_eq!(hash, hash2);
        // Different input produces different hash
        let hash3 = xxhash64_bytes(b"hello world!");
        assert_ne!(hash, hash3);
    }

    #[test]
    fn test_xxhash64_wabbajack_compatibility() {
        // Wabbajack (C#/.NET) uses BitConverter.GetBytes(long) which is
        // little-endian on x86/x64, then base64-encodes the result.
        // xxh64("hello world", seed=0) = 0x45ab6734b21e6968
        // LE bytes → base64 = "aGkesjRnq0U="
        let hash = xxhash64_bytes(b"hello world");
        assert_eq!(
            hash.0, "aGkesjRnq0U=",
            "Hash must match Wabbajack LE format"
        );

        // Verify round-trip through to_u64
        let val = hash.to_u64().unwrap();
        assert_eq!(val, 0x45ab6734b21e6968);

        // Verify from_u64 produces the same base64
        let reconstructed = WjHash::from_u64(0x45ab6734b21e6968);
        assert_eq!(reconstructed.0, "aGkesjRnq0U=");
    }

    #[test]
    fn test_xxhash64_file_streaming_matches_bytes() {
        // Regression: `xxhash64_file` used to slurp the whole file into a
        // `Vec<u8>` before hashing, OOMing on 10-40 GB modlist archives on
        // 16 GB Steam Decks. Verify the streaming implementation produces
        // the exact same base64 hash as the in-memory `xxhash64_bytes`
        // helper for the existing Wabbajack-compat fixture.
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::write(tmp.path(), b"hello world").expect("write");
        let from_file = xxhash64_file(tmp.path()).expect("hash file");
        assert_eq!(
            from_file.0, "aGkesjRnq0U=",
            "streaming xxhash64_file must match the Wabbajack LE base64 fixture"
        );
        // And it must equal the in-memory variant for the same bytes.
        let from_bytes = xxhash64_bytes(b"hello world");
        assert_eq!(from_file, from_bytes);
    }

    #[test]
    fn test_xxhash64_file_streams_multi_chunk() {
        // Use enough data to force more than one read() call through the
        // 1 MiB internal buffer, ensuring `Xxh64::update()` is being fed
        // chunks rather than a single slice.
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        // ~3 MiB of pseudo-random-looking but deterministic bytes
        let mut data = Vec::with_capacity(3 * 1024 * 1024);
        for i in 0..(3 * 1024 * 1024_u32) {
            data.push((i.wrapping_mul(2654435761) >> 24) as u8);
        }
        std::fs::write(tmp.path(), &data).expect("write");
        let from_file = xxhash64_file(tmp.path()).expect("hash file");
        let from_bytes = xxhash64_bytes(&data);
        assert_eq!(
            from_file, from_bytes,
            "streaming hash of multi-chunk file must equal in-memory hash"
        );
    }

    #[test]
    fn test_normalize_wj_path() {
        assert_eq!(
            normalize_wj_path(r"mods\SkyUI\SkyUI_5_2_SE.bsa"),
            "mods/SkyUI/SkyUI_5_2_SE.bsa"
        );
    }

    #[test]
    fn test_substitute_wj_path() {
        let result = substitute_wj_path(
            r"GAMEDIR\Data\scripts",
            Path::new("/bottles/skyrim/drive_c/Games/Skyrim"),
            Path::new("/bottles/skyrim/modorganizer"),
        );
        assert_eq!(result, "/bottles/skyrim/drive_c/Games/Skyrim/Data/scripts");
    }

    #[test]
    fn test_install_status_roundtrip() {
        assert_eq!(
            WjInstallStatus::from_str(WjInstallStatus::Downloading.as_str()),
            WjInstallStatus::Downloading
        );
        assert_eq!(
            WjInstallStatus::from_str(WjInstallStatus::Completed.as_str()),
            WjInstallStatus::Completed
        );
    }

    #[test]
    fn test_deserialize_nexus_state() {
        let json = r#"{
            "$type": "NexusDownloader+State, Wabbajack.Lib",
            "Game": "SkyrimSpecialEdition",
            "ModID": 12604,
            "FileID": 35407
        }"#;
        let state: WjArchiveState = serde_json::from_str(json).unwrap();
        match state {
            WjArchiveState::Nexus {
                game,
                mod_id,
                file_id,
            } => {
                assert_eq!(game, "SkyrimSpecialEdition");
                assert_eq!(mod_id, 12604);
                assert_eq!(file_id, 35407);
            }
            _ => panic!("Expected Nexus variant"),
        }
    }

    #[test]
    fn test_deserialize_http_state() {
        let json = r#"{
            "$type": "Http, Wabbajack.DTOs",
            "Url": "https://example.com/mod.zip",
            "Headers": []
        }"#;
        let state: WjArchiveState = serde_json::from_str(json).unwrap();
        match state {
            WjArchiveState::Http { url, headers } => {
                assert_eq!(url, "https://example.com/mod.zip");
                assert!(headers.is_empty());
            }
            _ => panic!("Expected Http variant"),
        }
    }

    #[test]
    fn test_deserialize_manual_state() {
        let json = r#"{
            "$type": "Manual, Wabbajack.DTOs",
            "Url": "https://example.com",
            "Prompt": "Download this file manually"
        }"#;
        let state: WjArchiveState = serde_json::from_str(json).unwrap();
        match state {
            WjArchiveState::Manual { url, prompt } => {
                assert_eq!(url, "https://example.com");
                assert_eq!(prompt, "Download this file manually");
            }
            _ => panic!("Expected Manual variant"),
        }
    }

    #[test]
    fn test_archive_state_source_type_name() {
        let state = WjArchiveState::Nexus {
            game: String::new(),
            mod_id: 0,
            file_id: 0,
        };
        assert_eq!(state.source_type_name(), "Nexus");

        let state = WjArchiveState::Mega { url: String::new() };
        assert_eq!(state.source_type_name(), "Mega");
    }

    #[test]
    fn test_deserialize_from_archive_directive() {
        let json = r#"{
            "$type": "FromArchive, Wabbajack.DTOs",
            "Hash": "eSIyd+KOG3s=",
            "Size": 12345,
            "To": "mods\\SkyUI\\SkyUI.esp",
            "ArchiveHashPath": {
                "BaseHash": "abc123==",
                "Parts": ["SkyUI_5_2_SE.bsa", "scripts/SkyUI.pex"]
            }
        }"#;
        let directive: WjDirective = serde_json::from_str(json).unwrap();
        match directive {
            WjDirective::FromArchive {
                hash,
                size,
                to,
                archive_hash_path,
            } => {
                assert_eq!(hash.0, "eSIyd+KOG3s=");
                assert_eq!(size, 12345);
                assert_eq!(to, "mods\\SkyUI\\SkyUI.esp");
                assert_eq!(archive_hash_path.base_hash.0, "abc123==");
                assert_eq!(archive_hash_path.parts.len(), 2);
            }
            _ => panic!("Expected FromArchive variant"),
        }
    }

    #[test]
    fn test_directive_accessors() {
        let directive = WjDirective::InlineFile {
            hash: WjHash("abc=".to_string()),
            size: 100,
            to: "output/file.txt".to_string(),
            source_data_id: "42".to_string(),
        };
        assert_eq!(directive.to_path(), "output/file.txt");
        assert_eq!(directive.hash().0, "abc=");
        assert_eq!(directive.size(), 100);
        assert_eq!(directive.kind_name(), "InlineFile");
    }
}
