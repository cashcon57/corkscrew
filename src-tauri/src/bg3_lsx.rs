//! BG3 LSX (Larian XML) parser and writer.
//!
//! Handles two LSX shapes:
//! - `modsettings.lsx` — mod load order (read + write)
//! - `meta.lsx` — mod identity metadata extracted from a `.pak` archive (read-only)
//!
//! Uses quick-xml events-based reader; no serde derivation (the structure is too
//! dynamic and the attribute-keyed schema doesn't map cleanly to structs).

use std::io::Write;
use std::path::Path;

// ─── Master UUIDs ────────────────────────────────────────────────────────────

/// GustavDev — development/debug version of the main game module. Must be first.
pub const MASTER_GUSTAV_DEV_UUID: &str = "28ac9ce2-2aba-8cda-b3b5-6e922f71b6b8";

/// Gustav — base game campaign module.
pub const MASTER_GUSTAV_UUID: &str = "991c9c7a-fb80-40cb-8f0d-b92d4e80e9b1";

/// SharedDev — shared developer content module.
pub const MASTER_SHARED_DEV_UUID: &str = "3d0c5ff8-c95d-c907-ff3e-34b204f1c630";

// ─── Types ───────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModSettings {
    pub version: LsxVersion,
    pub mods: Vec<ModEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModEntry {
    pub folder: String,
    pub md5: String,
    pub name: String,
    pub publish_handle: String,
    pub uuid: String,
    pub version64: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LsxVersion {
    pub major: u32,
    pub minor: u32,
    pub revision: u32,
    pub build: u32,
}

#[derive(Clone, Debug)]
pub struct ModuleInfo {
    pub folder: String,
    pub name: String,
    pub uuid: String,
    pub version64: String,
    pub author: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LsxError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed lsx: {0}")]
    Parse(String),
    #[error("not found: {0}")]
    NotFound(String),
}

// ─── Default impls ───────────────────────────────────────────────────────────

impl Default for LsxVersion {
    fn default() -> Self {
        LsxVersion {
            major: 0,
            minor: 0,
            revision: 0,
            build: 0,
        }
    }
}

impl Default for ModEntry {
    fn default() -> Self {
        ModEntry {
            folder: String::new(),
            md5: String::new(),
            name: String::new(),
            publish_handle: String::new(),
            uuid: String::new(),
            version64: String::new(),
        }
    }
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Returns true if the given UUID (case-insensitive) is one of the three Larian
/// master module UUIDs. Callers should refuse to reorder or remove master entries.
pub fn is_master_entry(uuid: &str) -> bool {
    let lower = uuid.to_lowercase();
    lower == MASTER_GUSTAV_DEV_UUID
        || lower == MASTER_GUSTAV_UUID
        || lower == MASTER_SHARED_DEV_UUID
}

/// Convert a `ModuleInfo` (from `meta.lsx`) into a `ModEntry` suitable for
/// insertion into `modsettings.lsx`.
///
/// MD5 is always `""` and PublishHandle is always `"0"` for community mods.
pub fn module_info_to_mod_entry(info: &ModuleInfo) -> ModEntry {
    ModEntry {
        folder: info.folder.clone(),
        md5: String::new(),
        name: info.name.clone(),
        publish_handle: "0".to_string(),
        uuid: info.uuid.clone(),
        version64: info.version64.clone(),
    }
}

/// Parse `modsettings.lsx` from `path`.
///
/// Returns `Ok(ModSettings)` with an empty `mods` list if the file is valid XML
/// but contains no `ModuleSettings` region — the game creates that region on
/// first launch.
pub fn read_modsettings(path: &Path) -> Result<ModSettings, LsxError> {
    let xml = std::fs::read_to_string(path)?;
    let mut reader = quick_xml::Reader::from_str(&xml);
    reader.config_mut().trim_text(true);

    let mut version = LsxVersion::default();
    let mut mods: Vec<ModEntry> = Vec::new();
    let mut in_module_settings_region = false;
    let mut in_mods_node = false;
    let mut current_entry: Option<ModEntry> = None;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(ref e)) => {
                match e.name().as_ref() {
                    b"version" => {
                        for attr in e.attributes().flatten() {
                            let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("").to_string();
                            let val_str = attr.unescape_value().unwrap_or_default().to_string();
                            let val = val_str.parse::<u32>().unwrap_or(0);
                            match key.as_str() {
                                "major" => version.major = val,
                                "minor" => version.minor = val,
                                "revision" => version.revision = val,
                                "build" => version.build = val,
                                _ => {}
                            }
                        }
                    }
                    b"region" => {
                        if attr_id_equals(e, "ModuleSettings") {
                            in_module_settings_region = true;
                        }
                    }
                    b"node" => {
                        if in_module_settings_region {
                            match attr_id_value(e).as_deref() {
                                Some("Mods") => in_mods_node = true,
                                Some("ModuleShortDesc") if in_mods_node => {
                                    current_entry = Some(ModEntry::default());
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(quick_xml::events::Event::Empty(ref e)) => {
                match e.name().as_ref() {
                    b"version" => {
                        for attr in e.attributes().flatten() {
                            let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("").to_string();
                            let val = attr.unescape_value().unwrap_or_default().parse::<u32>().unwrap_or(0);
                            match key.as_str() {
                                "major" => version.major = val,
                                "minor" => version.minor = val,
                                "revision" => version.revision = val,
                                "build" => version.build = val,
                                _ => {}
                            }
                        }
                    }
                    b"node" => {
                        if in_module_settings_region {
                            match attr_id_value(e).as_deref() {
                                Some("Mods") => in_mods_node = true,
                                Some("ModuleShortDesc") if in_mods_node => {
                                    // Self-closing node with no children — unlikely but valid.
                                    // Would produce an entry with empty uuid, so it gets dropped below.
                                    let entry = ModEntry::default();
                                    if !entry.uuid.is_empty() {
                                        mods.push(entry);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    b"attribute" => {
                        if let Some(entry) = current_entry.as_mut() {
                            let id = attr_value(e, "id");
                            let value = attr_value(e, "value").unwrap_or_default();
                            match id.as_deref() {
                                Some("Folder") => entry.folder = value,
                                Some("MD5") => entry.md5 = value,
                                Some("Name") => entry.name = value,
                                Some("PublishHandle") => entry.publish_handle = value,
                                Some("UUID") => entry.uuid = value,
                                Some("Version64") => entry.version64 = value,
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(quick_xml::events::Event::End(ref e)) => {
                match e.name().as_ref() {
                    b"node" => {
                        if let Some(entry) = current_entry.take() {
                            if !entry.uuid.is_empty() {
                                mods.push(entry);
                            }
                        }
                    }
                    b"region" => {
                        in_module_settings_region = false;
                        in_mods_node = false;
                    }
                    _ => {}
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(e) => return Err(LsxError::Parse(e.to_string())),
            _ => {}
        }
        buf.clear();
    }

    Ok(ModSettings { version, mods })
}

/// Write `settings` to `path` as canonical Larian XML.
///
/// Produces UTF-8 with Unix line endings, 4-space indent, XML declaration,
/// and the canonical `ModuleSettings` region structure.
pub fn write_modsettings(path: &Path, settings: &ModSettings) -> Result<(), LsxError> {
    let mut out = Vec::new();

    // XML declaration
    writeln!(out, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
    writeln!(out, "<save>")?;

    // Version element
    writeln!(
        out,
        r#"    <version major="{}" minor="{}" revision="{}" build="{}" />"#,
        settings.version.major,
        settings.version.minor,
        settings.version.revision,
        settings.version.build,
    )?;

    writeln!(out, r#"    <region id="ModuleSettings">"#)?;
    writeln!(out, r#"        <node id="root">"#)?;
    writeln!(out, "            <children>")?;
    writeln!(out, r#"                <node id="Mods">"#)?;
    writeln!(out, "                    <children>")?;

    for entry in &settings.mods {
        writeln!(out, r#"                        <node id="ModuleShortDesc">"#)?;
        writeln!(
            out,
            r#"                            <attribute id="Folder" type="LSString" value="{}" />"#,
            xml_escape(&entry.folder)
        )?;
        writeln!(
            out,
            r#"                            <attribute id="MD5" type="LSString" value="{}" />"#,
            xml_escape(&entry.md5)
        )?;
        writeln!(
            out,
            r#"                            <attribute id="Name" type="LSString" value="{}" />"#,
            xml_escape(&entry.name)
        )?;
        writeln!(
            out,
            r#"                            <attribute id="PublishHandle" type="uint64" value="{}" />"#,
            xml_escape(&entry.publish_handle)
        )?;
        writeln!(
            out,
            r#"                            <attribute id="UUID" type="guid" value="{}" />"#,
            xml_escape(&entry.uuid)
        )?;
        writeln!(
            out,
            r#"                            <attribute id="Version64" type="int64" value="{}" />"#,
            xml_escape(&entry.version64)
        )?;
        writeln!(out, "                        </node>")?;
    }

    writeln!(out, "                    </children>")?;
    writeln!(out, "                </node>")?;
    writeln!(out, "            </children>")?;
    writeln!(out, "        </node>")?;
    writeln!(out, "    </region>")?;
    writeln!(out, "</save>")?;

    std::fs::write(path, &out)?;
    Ok(())
}

/// Parse `meta.lsx` from `path` and extract the `ModuleInfo` node.
///
/// Returns `Err(LsxError::NotFound)` if no `ModuleInfo` node is present.
pub fn read_meta_lsx(path: &Path) -> Result<ModuleInfo, LsxError> {
    let xml = std::fs::read_to_string(path)?;
    read_meta_lsx_from_bytes(xml.as_bytes())
}

/// Parse `meta.lsx` from raw bytes (e.g. decompressed from a `.pak`).
///
/// This is the inner implementation shared by `read_meta_lsx` and the pak reader.
pub fn read_meta_lsx_from_bytes(bytes: &[u8]) -> Result<ModuleInfo, LsxError> {
    let xml = std::str::from_utf8(bytes).map_err(|e| LsxError::Parse(e.to_string()))?;
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut in_module_info = false;
    let mut buf = Vec::new();

    let mut folder = String::new();
    let mut name = String::new();
    let mut uuid = String::new();
    let mut version64 = String::new();
    let mut author: Option<String> = None;
    let mut description: Option<String> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(ref e)) => {
                if e.name().as_ref() == b"node" {
                    if let Some(id) = attr_id_value(e) {
                        if id == "ModuleInfo" {
                            in_module_info = true;
                        }
                    }
                }
            }
            Ok(quick_xml::events::Event::Empty(ref e)) => {
                if e.name().as_ref() == b"attribute" && in_module_info {
                    let id = attr_value(e, "id");
                    let value = attr_value(e, "value").unwrap_or_default();
                    match id.as_deref() {
                        Some("Folder") => folder = value,
                        Some("Name") => name = value,
                        Some("UUID") => uuid = value,
                        Some("Version64") => version64 = value,
                        Some("Author") => author = Some(value),
                        Some("Description") => description = Some(value),
                        _ => {}
                    }
                }
            }
            Ok(quick_xml::events::Event::End(ref e)) => {
                if e.name().as_ref() == b"node" && in_module_info {
                    // Exit ModuleInfo on its closing tag. We track the single
                    // opening node — when we see a matching end node while in_module_info
                    // is true we can stop.
                    in_module_info = false;
                    // If we have a UUID this node was ModuleInfo; we're done.
                    if !uuid.is_empty() {
                        break;
                    }
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(e) => return Err(LsxError::Parse(e.to_string())),
            _ => {}
        }
        buf.clear();
    }

    if uuid.is_empty() {
        return Err(LsxError::NotFound(
            "ModuleInfo node not found in meta.lsx".to_string(),
        ));
    }

    Ok(ModuleInfo {
        folder,
        name,
        uuid,
        version64,
        author,
        description,
    })
}

// ─── Internal helpers ────────────────────────────────────────────────────────

fn attr_id_value(e: &quick_xml::events::BytesStart) -> Option<String> {
    attr_value(e, "id")
}

fn attr_id_equals(e: &quick_xml::events::BytesStart, expected: &str) -> bool {
    attr_value(e, "id").as_deref() == Some(expected)
}

fn attr_value(e: &quick_xml::events::BytesStart, key: &str) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == key.as_bytes() {
            return Some(attr.unescape_value().ok()?.to_string());
        }
    }
    None
}

/// Escape the five XML predefined entities in attribute values.
///
/// quick-xml handles this when using its writer API, but since we're doing
/// manual string formatting we must escape ourselves.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_to_temp(contents: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        f
    }

    const MIN_MODSETTINGS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<save>
    <version major="4" minor="0" revision="9" build="319" />
    <region id="ModuleSettings">
        <node id="root">
            <children>
                <node id="Mods">
                    <children>
                        <node id="ModuleShortDesc">
                            <attribute id="Folder" type="LSString" value="GustavDev" />
                            <attribute id="MD5" type="LSString" value="" />
                            <attribute id="Name" type="LSString" value="GustavDev" />
                            <attribute id="PublishHandle" type="uint64" value="0" />
                            <attribute id="UUID" type="guid" value="28ac9ce2-2aba-8cda-b3b5-6e922f71b6b8" />
                            <attribute id="Version64" type="int64" value="36028797018963968" />
                        </node>
                    </children>
                </node>
            </children>
        </node>
    </region>
</save>
"#;

    #[test]
    fn read_modsettings_parses_minimal_fixture() {
        let f = write_to_temp(MIN_MODSETTINGS);
        let s = read_modsettings(f.path()).unwrap();
        assert_eq!(s.version.major, 4);
        assert_eq!(s.version.build, 319);
        assert_eq!(s.mods.len(), 1);
        assert_eq!(s.mods[0].folder, "GustavDev");
        assert_eq!(s.mods[0].uuid, "28ac9ce2-2aba-8cda-b3b5-6e922f71b6b8");
    }

    #[test]
    fn read_modsettings_preserves_mod_order() {
        let xml = MIN_MODSETTINGS.replace(
            r#"<node id="ModuleShortDesc">
                            <attribute id="Folder" type="LSString" value="GustavDev" />
                            <attribute id="MD5" type="LSString" value="" />
                            <attribute id="Name" type="LSString" value="GustavDev" />
                            <attribute id="PublishHandle" type="uint64" value="0" />
                            <attribute id="UUID" type="guid" value="28ac9ce2-2aba-8cda-b3b5-6e922f71b6b8" />
                            <attribute id="Version64" type="int64" value="36028797018963968" />
                        </node>"#,
            r#"<node id="ModuleShortDesc">
                            <attribute id="Folder" type="LSString" value="GustavDev" />
                            <attribute id="MD5" type="LSString" value="" />
                            <attribute id="Name" type="LSString" value="GustavDev" />
                            <attribute id="PublishHandle" type="uint64" value="0" />
                            <attribute id="UUID" type="guid" value="28ac9ce2-2aba-8cda-b3b5-6e922f71b6b8" />
                            <attribute id="Version64" type="int64" value="36028797018963968" />
                        </node>
                        <node id="ModuleShortDesc">
                            <attribute id="Folder" type="LSString" value="ModB" />
                            <attribute id="MD5" type="LSString" value="" />
                            <attribute id="Name" type="LSString" value="ModB" />
                            <attribute id="PublishHandle" type="uint64" value="0" />
                            <attribute id="UUID" type="guid" value="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb" />
                            <attribute id="Version64" type="int64" value="1" />
                        </node>
                        <node id="ModuleShortDesc">
                            <attribute id="Folder" type="LSString" value="ModC" />
                            <attribute id="MD5" type="LSString" value="" />
                            <attribute id="Name" type="LSString" value="ModC" />
                            <attribute id="PublishHandle" type="uint64" value="0" />
                            <attribute id="UUID" type="guid" value="cccccccc-cccc-cccc-cccc-cccccccccccc" />
                            <attribute id="Version64" type="int64" value="2" />
                        </node>"#,
        );
        let f = write_to_temp(&xml);
        let s = read_modsettings(f.path()).unwrap();
        assert_eq!(
            s.mods.iter().map(|m| m.folder.as_str()).collect::<Vec<_>>(),
            vec!["GustavDev", "ModB", "ModC"]
        );
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("modsettings.lsx");
        let s = ModSettings {
            version: LsxVersion {
                major: 4,
                minor: 0,
                revision: 9,
                build: 319,
            },
            mods: vec![
                ModEntry {
                    folder: "GustavDev".into(),
                    md5: "".into(),
                    name: "GustavDev".into(),
                    publish_handle: "0".into(),
                    uuid: "28ac9ce2-2aba-8cda-b3b5-6e922f71b6b8".into(),
                    version64: "36028797018963968".into(),
                },
                ModEntry {
                    folder: "MyMod".into(),
                    md5: "".into(),
                    name: "My Mod".into(),
                    publish_handle: "0".into(),
                    uuid: "12345678-1234-1234-1234-123456789abc".into(),
                    version64: "1".into(),
                },
            ],
        };
        write_modsettings(&path, &s).unwrap();
        let s2 = read_modsettings(&path).unwrap();
        assert_eq!(s, s2);
    }

    const META_LSX: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
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

    #[test]
    fn read_meta_lsx_extracts_module_info() {
        let f = write_to_temp(META_LSX);
        let info = read_meta_lsx(f.path()).unwrap();
        assert_eq!(info.folder, "TestMod");
        assert_eq!(info.name, "Test Mod");
        assert_eq!(info.uuid, "abcdef01-2345-6789-abcd-ef0123456789");
        assert_eq!(info.version64, "36028797018963968");
        assert_eq!(info.author.as_deref(), Some("ModAuthor"));
    }

    #[test]
    fn module_info_to_mod_entry_uses_canonical_defaults() {
        let info = ModuleInfo {
            folder: "Foo".into(),
            name: "Foo".into(),
            uuid: "u".into(),
            version64: "v".into(),
            author: None,
            description: None,
        };
        let entry = module_info_to_mod_entry(&info);
        assert_eq!(entry.md5, "");
        assert_eq!(entry.publish_handle, "0");
    }

    #[test]
    fn is_master_entry_recognizes_known_uuids() {
        assert!(is_master_entry(MASTER_GUSTAV_DEV_UUID));
        assert!(is_master_entry(
            MASTER_GUSTAV_DEV_UUID.to_uppercase().as_str()
        ));
        assert!(!is_master_entry("00000000-0000-0000-0000-000000000000"));
    }

    #[test]
    fn read_modsettings_returns_empty_for_missing_module_settings_region() {
        // Valid XML but no ModuleSettings region — return empty mods, default version.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<save>
    <region id="OtherRegion"><node id="x"/></region>
</save>
"#;
        let f = write_to_temp(xml);
        let s = read_modsettings(f.path()).unwrap();
        assert!(s.mods.is_empty());
    }

    #[test]
    fn read_meta_lsx_returns_not_found_for_missing_module_info_node() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<save>
    <region id="Config"><node id="root"><children></children></node></region>
</save>
"#;
        let f = write_to_temp(xml);
        let result = read_meta_lsx(f.path());
        assert!(matches!(result, Err(LsxError::NotFound(_))));
    }
}
