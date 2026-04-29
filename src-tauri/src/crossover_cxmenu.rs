//! Parser for CrossOver's `cxmenu.conf` shortcut registry.
//!
//! CrossOver writes a Mac-side INI file (`cxmenu.conf`, occasionally `cxmenu`
//! without an extension) at the bottle root that records *every* shortcut the
//! bottle exposes — including manually-added games that never get a `.lnk`
//! written into `drive_c`. This module parses that file and returns structured
//! entries so [`crate::crossover_shortcuts`] can surface those games.
//!
//! # Section-header encoding
//!
//! Each section header has the form `[PREFIX.ENCODED_WINDOWS_PATH]` where
//! PREFIX is a location label (`Desktop`, `StartMenu`, …) and ENCODED_WINDOWS_PATH
//! is the Windows path with the following substitutions applied by CrossOver:
//!
//! | Source character | Encoded as |
//! |---|---|
//! | `:` (colon) | `^3A` (or any `^XX` two-hex-digit sequence) |
//! | ` ` (space) | `+` |
//! | `\` (backslash) | `_` or `/` (both used as path-segment separators) |
//!
//! Example: `Desktop.C^3A_users_Public_Desktop/Steam.lnk`
//! → decoded path `C:\users\Public\Desktop\Steam.lnk`
//!
//! # Limits
//!
//! Entry parsing is capped at 10 000 sections to prevent pathological files
//! from consuming unbounded memory.

use std::path::Path;

/// Maximum number of entries we will parse from a single `cxmenu.conf` to
/// defend against pathologically large or malformed files.
const MAX_ENTRIES: usize = 10_000;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// One `[SECTION]` entry parsed from `cxmenu.conf`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CxmenuEntry {
    /// The decoded Windows-style path extracted from the section header
    /// (e.g. `C:\users\Public\Desktop\Steam.lnk`).
    pub windows_path: String,

    /// Value of the `"Shortcut"` key, if present.
    pub shortcut_name: Option<String>,

    /// Value of the `"StartupWMClass"` key — usually the `.exe` filename
    /// CrossOver expects the process to run as (e.g. `eldenring.exe`).
    pub startup_wm_class: Option<String>,

    /// Value of the `"Mode"` key (e.g. `"install"`, `"uninstall"`).
    pub mode: Option<String>,

    /// The raw section header text (everything between `[` and `]`), useful
    /// for diagnostics.
    pub raw_section: String,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse a `cxmenu.conf` (or `cxmenu`) file and return all valid entries.
///
/// The function is **error-tolerant**: a missing file, a read error, or any
/// malformed line is silently skipped. The caller always gets a (possibly
/// empty) vec — no `Result` needed at the call site.
///
/// Entries are capped at [`MAX_ENTRIES`] to avoid DoS from oversized files.
pub fn parse_cxmenu(path: &Path) -> Vec<CxmenuEntry> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    parse_cxmenu_str(&content)
}

/// Like [`parse_cxmenu`] but operates on an already-loaded string.
/// Exposed for testing without needing temp files.
pub fn parse_cxmenu_str(content: &str) -> Vec<CxmenuEntry> {
    let mut entries: Vec<CxmenuEntry> = Vec::new();

    // State machine: we track the current section and accumulate its
    // key=value pairs.
    let mut current_section: Option<String> = None;
    let mut shortcut_name: Option<String> = None;
    let mut startup_wm_class: Option<String> = None;
    let mut mode: Option<String> = None;

    /// Flush the current section into `entries`.
    macro_rules! flush {
        ($entries:expr, $current_section:expr,
         $shortcut_name:expr, $startup_wm_class:expr, $mode:expr) => {
            if let Some(raw) = $current_section.take() {
                // Strip the leading location prefix (e.g. "Desktop." or "StartMenu.").
                let encoded_path = strip_location_prefix(&raw);
                let windows_path = decode_cxmenu_path(encoded_path);
                $entries.push(CxmenuEntry {
                    windows_path,
                    shortcut_name: $shortcut_name.take(),
                    startup_wm_class: $startup_wm_class.take(),
                    mode: $mode.take(),
                    raw_section: raw,
                });
            }
        };
    }

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip blank lines and comments.
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            // New section header — flush any in-progress entry first.
            if entries.len() < MAX_ENTRIES {
                flush!(entries, current_section, shortcut_name, startup_wm_class, mode);
            } else {
                // Already at cap — clear state so we don't leak partial data.
                current_section = None;
                shortcut_name = None;
                startup_wm_class = None;
                mode = None;
                // Stop processing once we've hit the cap.
                break;
            }
            let header = trimmed[1..trimmed.len() - 1].to_string();
            current_section = Some(header);
            continue;
        }

        // Key-value line inside a section.
        if current_section.is_some() {
            if let Some((key, value)) = parse_kv_line(trimmed) {
                match key {
                    "Shortcut" => shortcut_name = Some(value.to_string()),
                    "StartupWMClass" => startup_wm_class = Some(value.to_string()),
                    "Mode" => mode = Some(value.to_string()),
                    _ => {} // Ignore Type, Icon, Arch, etc.
                }
            }
        }
    }

    // Flush the last in-progress section.
    if entries.len() < MAX_ENTRIES {
        flush!(entries, current_section, shortcut_name, startup_wm_class, mode);
    }

    entries
}

/// Strip the location-type prefix (e.g. `"Desktop."`, `"StartMenu."`) from a
/// raw section header, returning the remainder which is the encoded path.
///
/// CrossOver uses `Desktop.`, `StartMenu.`, and potentially others. We strip
/// everything up to and including the first `.`. If there is no `.`, the
/// entire string is treated as the path.
fn strip_location_prefix(raw: &str) -> &str {
    if let Some(dot_pos) = raw.find('.') {
        &raw[dot_pos + 1..]
    } else {
        raw
    }
}

/// Decode a CrossOver-encoded Windows path back to a conventional
/// backslash-separated path.
///
/// Encoding rules (applied left-to-right):
/// 1. `^XX` (caret + two uppercase or lowercase hex digits) → the character
///    with that code point.
/// 2. `+` → ` ` (space).
/// 3. `_` → `\` (path separator / backslash).
/// 4. `/` → `\` (also used as a path separator in section keys).
fn decode_cxmenu_path(encoded: &str) -> String {
    let mut out = String::with_capacity(encoded.len());
    let bytes = encoded.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];

        if b == b'^' && i + 2 < bytes.len() {
            // Attempt to decode two hex digits.
            let hi = hex_digit(bytes[i + 1]);
            let lo = hex_digit(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                let code = (h << 4) | l;
                // Only emit printable / reasonable ASCII. For non-ASCII code
                // points (unlikely in path context) fall back to '?'.
                if code < 128 {
                    out.push(code as char);
                } else {
                    out.push('?');
                }
                i += 3;
                continue;
            }
            // Not a valid escape — emit '^' literally.
            out.push('^');
            i += 1;
        } else if b == b'+' {
            out.push(' ');
            i += 1;
        } else if b == b'_' || b == b'/' {
            out.push('\\');
            i += 1;
        } else {
            out.push(b as char);
            i += 1;
        }
    }

    out
}

/// Convert a single ASCII hex digit byte to its numeric value, or `None`.
fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Parse a cxmenu key-value line of the form `"Key" = "Value"` or
/// `"Key" = "Value with spaces"`.
///
/// Returns `(key, value)` on success, `None` on parse failure. The returned
/// slices borrow from `line`.
fn parse_kv_line(line: &str) -> Option<(&str, &str)> {
    // Strip optional leading whitespace (already trimmed by caller, but be safe).
    let line = line.trim();

    // Key must start and end with `"`.
    let line = line.strip_prefix('"')?;
    let (key, rest) = line.split_once('"')?;

    // Consume optional whitespace + `=` + optional whitespace.
    let rest = rest.trim();
    let rest = rest.strip_prefix('=')?;
    let rest = rest.trim();

    // Value is enclosed in `"…"`.
    let rest = rest.strip_prefix('"')?;
    let (value, _) = rest.split_once('"')?;

    Some((key, value))
}

// ---------------------------------------------------------------------------
// Helper: look for cxmenu.conf (with or without extension) at the bottle root
// ---------------------------------------------------------------------------

/// Return the path to the `cxmenu.conf` (or `cxmenu`) file at the bottle root,
/// or `None` if neither name is present.
pub fn find_cxmenu_file(bottle_root: &Path) -> Option<std::path::PathBuf> {
    let with_ext = bottle_root.join("cxmenu.conf");
    if with_ext.is_file() {
        return Some(with_ext);
    }
    let without_ext = bottle_root.join("cxmenu");
    if without_ext.is_file() {
        return Some(without_ext);
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ----- decode_cxmenu_path -----------------------------------------------

    #[test]
    fn decode_colon_hex_escape() {
        // `C^3A_users_Public_Desktop/Steam.lnk` → `C:\users\Public\Desktop\Steam.lnk`
        assert_eq!(
            decode_cxmenu_path("C^3A_users_Public_Desktop/Steam.lnk"),
            r"C:\users\Public\Desktop\Steam.lnk"
        );
    }

    #[test]
    fn decode_plus_space() {
        // `Start+Menu` → `Start Menu`
        assert_eq!(decode_cxmenu_path("Start+Menu"), "Start Menu");
    }

    #[test]
    fn decode_full_start_menu_path() {
        let encoded =
            "C^3A_ProgramData_Microsoft_Windows_Start+Menu/Programs/Steam/Steam.lnk";
        let expected = r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Steam\Steam.lnk";
        assert_eq!(decode_cxmenu_path(encoded), expected);
    }

    #[test]
    fn decode_literal_caret_when_not_hex_escape() {
        // `^ZZ` is not a valid hex escape → `^` emitted literally.
        let result = decode_cxmenu_path("^ZZ");
        assert!(result.starts_with('^'));
    }

    // ----- strip_location_prefix --------------------------------------------

    #[test]
    fn strip_desktop_prefix() {
        assert_eq!(
            strip_location_prefix("Desktop.C^3A_users_Public_Desktop/Steam.lnk"),
            "C^3A_users_Public_Desktop/Steam.lnk"
        );
    }

    #[test]
    fn strip_startmenu_prefix() {
        assert_eq!(
            strip_location_prefix(
                "StartMenu.C^3A_ProgramData_Microsoft_Windows_Start+Menu/Programs/Steam/Steam.lnk"
            ),
            "C^3A_ProgramData_Microsoft_Windows_Start+Menu/Programs/Steam/Steam.lnk"
        );
    }

    #[test]
    fn strip_no_dot_returns_whole_string() {
        assert_eq!(strip_location_prefix("SomeNoDot"), "SomeNoDot");
    }

    // ----- parse_kv_line ----------------------------------------------------

    #[test]
    fn parse_kv_basic() {
        let (k, v) = parse_kv_line(r#""StartupWMClass" = "eldenring.exe""#).unwrap();
        assert_eq!(k, "StartupWMClass");
        assert_eq!(v, "eldenring.exe");
    }

    #[test]
    fn parse_kv_no_spaces_around_eq() {
        // Some tools omit spaces around the `=` — handle gracefully.
        let (k, v) = parse_kv_line(r#""Mode"="install""#).unwrap();
        assert_eq!(k, "Mode");
        assert_eq!(v, "install");
    }

    #[test]
    fn parse_kv_returns_none_for_garbage() {
        assert!(parse_kv_line("not a kv line").is_none());
        assert!(parse_kv_line("").is_none());
        assert!(parse_kv_line("[section]").is_none());
    }

    // ----- parse_cxmenu_str (integration) -----------------------------------

    #[test]
    fn parse_single_entry_with_all_fields() {
        let input = r#"
[Desktop.C^3A_users_Public_Desktop/Steam.lnk]
"Type" = "Windows"
"Icon" = "5026_steam.0"
"Shortcut" = "steam"
"Mode" = "install"
"StartupWMClass" = "steam.exe"
"Arch" = "x86_64"
"#;
        let entries = parse_cxmenu_str(input);
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.windows_path, r"C:\users\Public\Desktop\Steam.lnk");
        assert_eq!(e.shortcut_name.as_deref(), Some("steam"));
        assert_eq!(e.startup_wm_class.as_deref(), Some("steam.exe"));
        assert_eq!(e.mode.as_deref(), Some("install"));
        assert_eq!(
            e.raw_section,
            "Desktop.C^3A_users_Public_Desktop/Steam.lnk"
        );
    }

    #[test]
    fn parse_multiple_entries() {
        let input = r#"
[Desktop.C^3A_users_Public_Desktop/Steam.lnk]
"StartupWMClass" = "steam.exe"
"Mode" = "install"

[StartMenu.C^3A_ProgramData_Microsoft_Windows_Start+Menu/Programs/Steam/Steam.lnk]
"Shortcut" = "steam"
"StartupWMClass" = "steam.exe"
"Mode" = "install"
"#;
        let entries = parse_cxmenu_str(input);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].windows_path,
            r"C:\users\Public\Desktop\Steam.lnk"
        );
        assert_eq!(
            entries[1].windows_path,
            r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Steam\Steam.lnk"
        );
    }

    #[test]
    fn parse_entry_without_startup_wm_class_does_not_crash() {
        let input = r#"
[StartMenu.C^3A_users_crossover_Desktop/The+Midnight+Walk.url]
"Type" = "Windows"
"Mode" = "install"
"#;
        let entries = parse_cxmenu_str(input);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].startup_wm_class.is_none());
        assert!(entries[0].shortcut_name.is_none());
        assert_eq!(entries[0].mode.as_deref(), Some("install"));
    }

    #[test]
    fn parse_comments_and_blank_lines_skipped() {
        let input = r#"
# This is a comment
; So is this

[Desktop.C^3A_users_Public_Desktop/Steam.lnk]
"StartupWMClass" = "steam.exe"

# Another comment
"Mode" = "install"
"#;
        let entries = parse_cxmenu_str(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].startup_wm_class.as_deref(), Some("steam.exe"));
        assert_eq!(entries[0].mode.as_deref(), Some("install"));
    }

    #[test]
    fn parse_malformed_file_returns_empty() {
        // A file that has no valid sections at all.
        let input = "not an ini file at all!!!!\n$$$garbage###";
        let entries = parse_cxmenu_str(input);
        // No sections found → empty (lines without `[` header are just ignored).
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_empty_file_returns_empty() {
        let entries = parse_cxmenu_str("");
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_missing_file_returns_empty() {
        let entries = parse_cxmenu(Path::new("/tmp/corkscrew_test_nonexistent_cxmenu.conf_zzz"));
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_caps_at_max_entries() {
        // Build a synthetic file with MAX_ENTRIES + 1 sections.
        let mut content = String::new();
        for i in 0..=(MAX_ENTRIES) {
            content.push_str(&format!(
                "[Desktop.C^3A_users_Public_Desktop/Game{}.lnk]\n",
                i
            ));
            content.push_str("\"Mode\" = \"install\"\n");
            content.push_str("\"StartupWMClass\" = \"game.exe\"\n\n");
        }
        let entries = parse_cxmenu_str(&content);
        assert_eq!(entries.len(), MAX_ENTRIES);
    }

    #[test]
    fn find_cxmenu_file_with_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cxmenu.conf");
        fs::write(&path, "[Desktop.test]\n").unwrap();
        assert_eq!(find_cxmenu_file(tmp.path()), Some(path));
    }

    #[test]
    fn find_cxmenu_file_without_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cxmenu");
        fs::write(&path, "[Desktop.test]\n").unwrap();
        assert_eq!(find_cxmenu_file(tmp.path()), Some(path));
    }

    #[test]
    fn find_cxmenu_file_prefers_with_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let with_ext = tmp.path().join("cxmenu.conf");
        let without_ext = tmp.path().join("cxmenu");
        fs::write(&with_ext, "[Desktop.test]\n").unwrap();
        fs::write(&without_ext, "[Desktop.other]\n").unwrap();
        assert_eq!(find_cxmenu_file(tmp.path()), Some(with_ext));
    }

    #[test]
    fn find_cxmenu_file_returns_none_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(find_cxmenu_file(tmp.path()).is_none());
    }

    /// Manual smoke test against the real CrossOver Steam bottle.
    ///
    /// Run with: `cargo test --lib -- crossover_cxmenu::tests::real_steam_bottle --ignored`
    ///
    /// This test is `#[ignore]` so it does not run in CI or the normal test
    /// suite. It verifies that the parser handles a real `cxmenu.conf` without
    /// panicking and that Steam's `steam.exe` entry is surfaced.
    #[test]
    #[ignore]
    fn real_steam_bottle_smoke() {
        let steam_bottle = std::path::Path::new(
            "/Users/cashconway/Library/Application Support/CrossOver/Bottles/Steam",
        );
        let Some(cxmenu_path) = find_cxmenu_file(steam_bottle) else {
            eprintln!("No cxmenu.conf found at {}", steam_bottle.display());
            return;
        };

        let entries = parse_cxmenu(&cxmenu_path);
        println!(
            "Parsed {} entries from {}",
            entries.len(),
            cxmenu_path.display()
        );
        for e in &entries {
            println!(
                "  path={:?} wm_class={:?} shortcut={:?}",
                e.windows_path, e.startup_wm_class, e.shortcut_name
            );
        }

        assert!(!entries.is_empty(), "Expected at least one entry from Steam cxmenu.conf");

        // The Steam bottle must have at least one entry with steam.exe.
        let has_steam = entries
            .iter()
            .any(|e| e.startup_wm_class.as_deref() == Some("steam.exe"));
        assert!(has_steam, "Expected steam.exe entry in Steam bottle cxmenu.conf");
    }
}
