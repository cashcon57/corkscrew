//! Static index mapping Steam app IDs to upstream Vortex extensions.
//!
//! Vortex itself does not surface "is there an extension for this Steam game?"
//! when scanning installed titles — Corkscrew does. This module embeds a curated
//! JSON file (`data/vortex_extension_index.json`) at compile time and exposes
//! a fast lookup keyed by Steam app ID.
//!
//! Why a static, embedded list (rather than hitting GitHub at runtime)?
//! - The Steam scanner runs on every bottle detection pass and must be fast.
//! - The upstream extension set churns slowly; refreshing the JSON is a
//!   maintenance task, not a hot path.
//! - Extracting Steam app IDs from each extension's `index.js` is a one-time
//!   operation we don't want to repeat per scan.
//!
//! When an entry exists for a detected (but unregistered) Steam game,
//! [`game_registry`] surfaces a [`VortexExtensionSuggestion`] so the frontend
//! can offer a one-click install via the existing
//! `vortex_fetch_extension(vortex_dir_name)` Tauri command.
//!
//! To refresh the index: re-run the maintenance script (see commit history)
//! and edit `data/vortex_extension_index.json` in place. The file is parsed
//! once on first access and cached in a `OnceLock`; malformed JSON will
//! `panic` at first access (caught immediately by the test
//! [`tests::index_parses_successfully`] in CI).

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Raw JSON embedded at compile time.
const INDEX_JSON: &str = include_str!("../data/vortex_extension_index.json");

/// One Vortex extension entry, keyed by Steam app ID.
///
/// Matches the schema in `data/vortex_extension_index.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExtensionEntry {
    /// Stable identifier for this entry. Usually equals `vortex_dir_name`,
    /// but may be suffixed (e.g. `game-witcher3-goty`) when one extension
    /// covers multiple Steam app IDs (different editions of the same game).
    pub id: String,
    /// Human-readable display name (e.g. `"The Witcher 3: Wild Hunt"`).
    pub name: String,
    /// Steam application ID as a string (matches the format used by the
    /// appmanifest scanner). Always present.
    pub steam_app_id: String,
    /// Nexus Mods URL slug (e.g. `"witcher3"`). May not always match the
    /// Vortex extension's directory name.
    pub nexus_slug: String,
    /// Directory name in `Nexus-Mods/vortex-games` repo
    /// (e.g. `"game-witcher3"`). This is the `game_id` argument expected by
    /// [`crate::vortex_fetcher::fetch_extension`].
    pub vortex_dir_name: String,
}

/// Top-level index document.
#[derive(Debug, Deserialize)]
struct IndexDocument {
    #[allow(dead_code)]
    schema_version: u32,
    #[allow(dead_code)]
    #[serde(default)]
    generated_at: String,
    extensions: Vec<ExtensionEntry>,
}

/// Parse the embedded JSON once and return a static reference.
fn entries() -> &'static [ExtensionEntry] {
    static ENTRIES: OnceLock<Vec<ExtensionEntry>> = OnceLock::new();
    ENTRIES
        .get_or_init(|| {
            let doc: IndexDocument = serde_json::from_str(INDEX_JSON)
                .expect("vortex_extension_index.json is malformed");
            doc.extensions
        })
        .as_slice()
}

/// Look up the Vortex extension entry for a given Steam app ID.
///
/// Returns `None` if no extension matches. The lookup is a linear scan over
/// roughly 80 entries — small enough not to warrant a HashMap.
pub fn lookup_extension_for_steam_appid(appid: &str) -> Option<&'static ExtensionEntry> {
    if appid.is_empty() {
        return None;
    }
    entries().iter().find(|e| e.steam_app_id == appid)
}

/// Return the full list of indexed extensions (for diagnostics / UI).
pub fn all_entries() -> &'static [ExtensionEntry] {
    entries()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_parses_successfully() {
        let all = all_entries();
        assert!(
            !all.is_empty(),
            "vortex_extension_index.json parsed to zero entries"
        );
        // Sanity: required fields populated for every entry.
        for e in all {
            assert!(!e.id.is_empty(), "entry has empty id: {:?}", e);
            assert!(!e.name.is_empty(), "entry has empty name: {:?}", e);
            assert!(
                !e.steam_app_id.is_empty(),
                "entry has empty steam_app_id: {:?}",
                e
            );
            assert!(
                e.steam_app_id.chars().all(|c| c.is_ascii_digit()),
                "non-numeric steam_app_id: {:?}",
                e
            );
            assert!(
                e.vortex_dir_name.starts_with("game-"),
                "vortex_dir_name should start with 'game-': {:?}",
                e
            );
        }
    }

    #[test]
    fn lookup_finds_cyberpunk2077() {
        let entry = lookup_extension_for_steam_appid("1091500")
            .expect("Cyberpunk 2077 (1091500) should be indexed");
        assert_eq!(entry.vortex_dir_name, "game-cyberpunk2077");
        assert_eq!(entry.nexus_slug, "cyberpunk2077");
    }

    #[test]
    fn lookup_finds_witcher3() {
        let entry = lookup_extension_for_steam_appid("292030")
            .expect("Witcher 3 (292030) should be indexed");
        assert_eq!(entry.vortex_dir_name, "game-witcher3");
        assert!(entry.name.contains("Witcher"));
    }

    #[test]
    fn lookup_returns_none_for_unknown_appid() {
        assert!(lookup_extension_for_steam_appid("999999999").is_none());
        assert!(lookup_extension_for_steam_appid("not-a-number").is_none());
        assert!(lookup_extension_for_steam_appid("").is_none());
    }

    #[test]
    fn multiple_entries_can_share_vortex_dir() {
        // Witcher 3 + Witcher 3 GOTY both point at game-witcher3.
        let base = lookup_extension_for_steam_appid("292030").unwrap();
        let goty = lookup_extension_for_steam_appid("499450").unwrap();
        assert_eq!(base.vortex_dir_name, goty.vortex_dir_name);
        assert_ne!(base.id, goty.id);
    }

    #[test]
    fn no_duplicate_steam_app_ids() {
        let all = all_entries();
        let mut seen: Vec<&str> = Vec::with_capacity(all.len());
        for e in all {
            assert!(
                !seen.contains(&e.steam_app_id.as_str()),
                "duplicate steam_app_id: {}",
                e.steam_app_id
            );
            seen.push(&e.steam_app_id);
        }
    }
}
