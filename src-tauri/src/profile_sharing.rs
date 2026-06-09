//! Profile code sharing: compact, shareable codes that encode mod profiles.
//!
//! Generates short codes like `CRKS-7x9Km-4pQw...` from mod profiles using
//! JSON → zstd compression → base64url encoding. Codes are fully offline —
//! no cloud service required.

use base64::Engine;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A compact representation of a mod profile for sharing.
/// This is the minimal set of fields needed to reproduce a profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedProfile {
    /// Format version for forward compatibility.
    pub version: u32,
    /// Corkscrew game_id.
    pub game_id: String,
    /// Human-readable game name.
    pub game_name: String,
    /// Mods in this profile (ordered by priority).
    pub mods: Vec<SharedMod>,
    /// Plugin load order (filenames only).
    pub plugins: Vec<SharedPlugin>,
}

/// Minimal mod reference in a shared profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedMod {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub enabled: bool,
    pub priority: i32,
    /// NexusMods mod ID for automatic download resolution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nexus_mod_id: Option<i64>,
    /// NexusMods file ID for exact file matching.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nexus_file_id: Option<i64>,
}

/// Minimal plugin reference in a shared profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedPlugin {
    pub filename: String,
    pub enabled: bool,
}

/// The prefix used for all Corkscrew profile codes.
const CODE_PREFIX: &str = "CRKS";
/// Current profile code format version.
const FORMAT_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Encode a shared profile into a compact code string.
///
/// Pipeline: JSON → zstd compress → base64url → chunked with dashes.
pub fn encode_profile(profile: &SharedProfile) -> Result<String, String> {
    // Serialize to JSON (compact, no pretty-printing)
    let json =
        serde_json::to_vec(profile).map_err(|e| format!("JSON serialization failed: {}", e))?;

    // Compress with zstd (level 3 = good balance of speed vs ratio)
    let compressed =
        zstd::encode_all(json.as_slice(), 3).map_err(|e| format!("Compression failed: {}", e))?;

    // Encode as STANDARD base64 (uses +/ instead of -_ so we can use - as separator)
    let b64 = base64::engine::general_purpose::STANDARD_NO_PAD.encode(&compressed);

    // Chunk into 5-char groups with dashes, prefixed with CRKS
    // STANDARD base64 uses A-Z a-z 0-9 + / (no dashes), so dashes are safe separators
    let chunks: Vec<&str> = b64
        .as_bytes()
        .chunks(5)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect();

    Ok(format!("{}-{}", CODE_PREFIX, chunks.join("-")))
}

/// Decode a profile code string back into a SharedProfile.
///
/// Pipeline: strip prefix + dashes → base64url decode → zstd decompress → JSON parse.
pub fn decode_profile(code: &str) -> Result<SharedProfile, String> {
    // Strip prefix
    let stripped = code
        .strip_prefix(&format!("{}-", CODE_PREFIX))
        .ok_or_else(|| format!("Invalid code: must start with '{}-'", CODE_PREFIX))?;

    // Remove dashes and whitespace to reconstruct base64
    // STANDARD base64 uses +/ (not -_), so dashes are safe to strip
    let b64: String = stripped
        .chars()
        .filter(|c| !matches!(*c, '-' | ' ' | '\n' | '\r'))
        .collect();

    // Decode STANDARD base64 with lenient padding
    use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
    let decoder = GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );
    let compressed = decoder
        .decode(b64.as_bytes())
        .map_err(|e| format!("Base64 decode failed: {}", e))?;

    // Decompress zstd
    let json = zstd::decode_all(compressed.as_slice())
        .map_err(|e| format!("Decompression failed: {}", e))?;

    // Parse JSON
    let profile: SharedProfile =
        serde_json::from_slice(&json).map_err(|e| format!("JSON parse failed: {}", e))?;

    // Validate format version
    if profile.version > FORMAT_VERSION {
        return Err(format!(
            "Profile code version {} is newer than supported version {}. Please update Corkscrew.",
            profile.version, FORMAT_VERSION
        ));
    }

    Ok(profile)
}

/// Create a SharedProfile from installed mod data.
///
/// This converts the full InstalledMod data into the minimal SharedMod format
/// needed for profile sharing.
pub fn create_shared_profile(
    game_id: &str,
    game_name: &str,
    mods: &[crate::database::InstalledMod],
    plugins: &[(String, bool)], // (filename, enabled)
) -> SharedProfile {
    let shared_mods: Vec<SharedMod> = mods
        .iter()
        .map(|m| SharedMod {
            name: m.name.clone(),
            version: Some(m.version.clone()),
            enabled: m.enabled,
            priority: m.install_priority,
            nexus_mod_id: m.nexus_mod_id,
            nexus_file_id: m.nexus_file_id,
        })
        .collect();

    let shared_plugins: Vec<SharedPlugin> = plugins
        .iter()
        .map(|(name, enabled)| SharedPlugin {
            filename: name.clone(),
            enabled: *enabled,
        })
        .collect();

    SharedProfile {
        version: FORMAT_VERSION,
        game_id: game_id.to_string(),
        game_name: game_name.to_string(),
        mods: shared_mods,
        plugins: shared_plugins,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_length() {
        let profile = sample_profile();
        let json = serde_json::to_vec(&profile).unwrap();
        let compressed = zstd::encode_all(json.as_slice(), 3).unwrap();
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&compressed);
        // base64 no-pad output should be len % 4 in {0, 2, 3}, never 1
        let rem = b64.len() % 4;
        assert_ne!(
            rem,
            1,
            "base64 NO_PAD should never produce len%4==1, got len={}",
            b64.len()
        );
    }

    fn sample_profile() -> SharedProfile {
        SharedProfile {
            version: 1,
            game_id: "skyrimse".into(),
            game_name: "Skyrim Special Edition".into(),
            mods: vec![
                SharedMod {
                    name: "SkyUI".into(),
                    version: Some("5.2SE".into()),
                    enabled: true,
                    priority: 1,
                    nexus_mod_id: Some(12604),
                    nexus_file_id: Some(35407),
                },
                SharedMod {
                    name: "USSEP".into(),
                    version: Some("4.2.9b".into()),
                    enabled: true,
                    priority: 0,
                    nexus_mod_id: Some(266),
                    nexus_file_id: None,
                },
            ],
            plugins: vec![
                SharedPlugin {
                    filename: "Skyrim.esm".into(),
                    enabled: true,
                },
                SharedPlugin {
                    filename: "SkyUI_SE.esp".into(),
                    enabled: true,
                },
            ],
        }
    }

    #[test]
    fn roundtrip_encode_decode() {
        let profile = sample_profile();
        let code = encode_profile(&profile).unwrap();

        assert!(code.starts_with("CRKS-"), "Code should start with CRKS-");
        assert!(code.contains('-'), "Code should contain dashes");

        let decoded = decode_profile(&code).unwrap();
        assert_eq!(decoded.game_id, "skyrimse");
        assert_eq!(decoded.mods.len(), 2);
        assert_eq!(decoded.mods[0].name, "SkyUI");
        assert_eq!(decoded.mods[0].nexus_mod_id, Some(12604));
        assert_eq!(decoded.plugins.len(), 2);
    }

    #[test]
    fn decode_rejects_invalid_prefix() {
        assert!(decode_profile("INVALID-code").is_err());
    }

    #[test]
    fn decode_rejects_corrupt_data() {
        assert!(decode_profile("CRKS-AAAA-BBBB").is_err());
    }

    #[test]
    fn code_is_reasonably_compact() {
        let profile = sample_profile();
        let code = encode_profile(&profile).unwrap();
        // A 2-mod profile should compress well below 500 chars
        assert!(code.len() < 500, "Code too long: {} chars", code.len());
    }

    #[test]
    fn empty_profile_roundtrips() {
        let profile = SharedProfile {
            version: 1,
            game_id: "skyrimse".into(),
            game_name: "Skyrim SE".into(),
            mods: vec![],
            plugins: vec![],
        };
        let code = encode_profile(&profile).unwrap();
        let decoded = decode_profile(&code).unwrap();
        assert_eq!(decoded.mods.len(), 0);
    }
}
