//! FOMOD installer XML parser.
//!
//! Ported from the legacy Python FOMOD parser. Parses `ModuleConfig.xml` files
//! produced by FOMOD installer packs to extract installation options, file
//! mappings, and step metadata.
//!
//! The parser uses `quick_xml`'s event-based reader API to walk the XML tree
//! and build an in-memory [`FomodInstaller`] struct that the rest of the
//! application can use to drive the mod installation wizard.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// A single file or folder mapping within a FOMOD option.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FomodFile {
    /// Source path inside the mod archive.
    pub source: String,
    /// Destination path relative to the game's data directory.
    pub destination: String,
    /// Installation priority (higher wins on conflict).
    pub priority: i32,
    /// Whether this entry represents a folder rather than a single file.
    pub is_folder: bool,
}

/// A single selectable option within a FOMOD installer group.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FomodOption {
    /// Display name of the option.
    pub name: String,
    /// Description text shown to the user.
    pub description: String,
    /// Optional path to a preview image (relative to the FOMOD directory).
    pub image: Option<String>,
    /// Files installed when this option is selected.
    pub files: Vec<FomodFile>,
    /// Type descriptor controlling default selection behaviour.
    /// Common values: `"Optional"`, `"Required"`, `"Recommended"`,
    /// `"NotUsable"`, `"CouldBeUsable"`.
    pub type_descriptor: String,
}

/// A group of related options within a FOMOD installer step.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FomodGroup {
    /// Display name of the group.
    pub name: String,
    /// Selection type: `"SelectExactlyOne"`, `"SelectAtMostOne"`,
    /// `"SelectAtLeastOne"`, `"SelectAll"`, or `"SelectAny"`.
    pub group_type: String,
    /// The options belonging to this group.
    pub options: Vec<FomodOption>,
}

/// A single step (page) in the FOMOD installer wizard.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FomodStep {
    /// Display name of the step.
    pub name: String,
    /// Groups presented on this step.
    pub groups: Vec<FomodGroup>,
}

/// Top-level FOMOD installer structure parsed from `ModuleConfig.xml`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FomodInstaller {
    /// Module name declared in the XML.
    pub module_name: String,
    /// Files that are always installed regardless of user selections.
    pub required_files: Vec<FomodFile>,
    /// Ordered list of installer steps.
    pub steps: Vec<FomodStep>,
}

// ---------------------------------------------------------------------------
// XML parsing helpers
// ---------------------------------------------------------------------------

/// Read the text content of the current element from the reader.
fn read_text_content(reader: &mut Reader<&[u8]>, buf: &mut Vec<u8>) -> String {
    let mut text = String::new();
    loop {
        buf.clear();
        match reader.read_event_into(buf) {
            Ok(Event::Text(e)) => {
                if let Ok(t) = e.unescape() {
                    text.push_str(&t);
                }
            }
            Ok(Event::CData(e)) => {
                // CData content does not need unescaping.
                text.push_str(&String::from_utf8_lossy(&e));
            }
            Ok(Event::End(_)) | Ok(Event::Eof) => break,
            _ => {}
        }
    }
    text.trim().to_string()
}

/// Extract a named attribute value from a `BytesStart` tag.
fn get_attr(tag: &quick_xml::events::BytesStart<'_>, name: &str) -> Option<String> {
    for attr in tag.attributes().flatten() {
        if attr.key.as_ref() == name.as_bytes() {
            return String::from_utf8(attr.value.to_vec()).ok();
        }
    }
    None
}

/// Parse a `<file>` or `<folder>` element into a [`FomodFile`].
fn parse_file_element(tag: &quick_xml::events::BytesStart<'_>, is_folder: bool) -> FomodFile {
    let source = get_attr(tag, "source").unwrap_or_default();
    let destination = get_attr(tag, "destination").unwrap_or_default();
    let priority = get_attr(tag, "priority")
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    FomodFile {
        source,
        destination,
        priority,
        is_folder,
    }
}

/// Parse a `<files>` block and return its file/folder entries.
fn parse_files_block(reader: &mut Reader<&[u8]>, buf: &mut Vec<u8>) -> Vec<FomodFile> {
    let mut files = Vec::new();
    let mut depth = 1u32;
    loop {
        buf.clear();
        match reader.read_event_into(buf) {
            Ok(Event::Start(ref e)) => {
                depth += 1;
                let local = e.local_name();
                match local.as_ref() {
                    b"file" => files.push(parse_file_element(e, false)),
                    b"folder" => files.push(parse_file_element(e, true)),
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                let local = e.local_name();
                match local.as_ref() {
                    b"file" => files.push(parse_file_element(e, false)),
                    b"folder" => files.push(parse_file_element(e, true)),
                    _ => {}
                }
            }
            Ok(Event::End(_)) => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    files
}

/// Parse a `<plugin>` element into a [`FomodOption`].
fn parse_plugin(reader: &mut Reader<&[u8]>, buf: &mut Vec<u8>, name: String) -> FomodOption {
    let mut option = FomodOption {
        name,
        description: String::new(),
        image: None,
        files: Vec::new(),
        type_descriptor: "Optional".to_string(),
    };

    let mut depth = 1u32;
    loop {
        buf.clear();
        match reader.read_event_into(buf) {
            Ok(Event::Start(ref e)) => {
                depth += 1;
                let local_name = e.local_name();
                match local_name.as_ref() {
                    b"description" => {
                        option.description = read_text_content(reader, buf);
                        // read_text_content consumes the End event
                        depth -= 1;
                    }
                    b"files" => {
                        option.files = parse_files_block(reader, buf);
                        // parse_files_block consumes the End event
                        depth -= 1;
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                let local_name = e.local_name();
                match local_name.as_ref() {
                    b"image" => {
                        option.image = get_attr(e, "path");
                    }
                    b"type" => {
                        if let Some(name) = get_attr(e, "name") {
                            option.type_descriptor = name;
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(_)) => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    option
}

/// Parse a `<group>` element into a [`FomodGroup`].
fn parse_group(
    reader: &mut Reader<&[u8]>,
    buf: &mut Vec<u8>,
    name: String,
    group_type: String,
) -> FomodGroup {
    let mut group = FomodGroup {
        name,
        group_type,
        options: Vec::new(),
    };

    let mut depth = 1u32;
    loop {
        buf.clear();
        match reader.read_event_into(buf) {
            Ok(Event::Start(ref e)) => {
                depth += 1;
                let local_name = e.local_name();
                if local_name.as_ref() == b"plugin" {
                    let plugin_name = get_attr(e, "name").unwrap_or_default();
                    let option = parse_plugin(reader, buf, plugin_name);
                    group.options.push(option);
                    // parse_plugin consumes the End event
                    depth -= 1;
                }
            }
            Ok(Event::End(_)) => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    group
}

/// Parse an `<installStep>` element into a [`FomodStep`].
fn parse_step(reader: &mut Reader<&[u8]>, buf: &mut Vec<u8>, name: String) -> FomodStep {
    let mut step = FomodStep {
        name,
        groups: Vec::new(),
    };

    let mut depth = 1u32;
    loop {
        buf.clear();
        match reader.read_event_into(buf) {
            Ok(Event::Start(ref e)) => {
                depth += 1;
                let local_name = e.local_name();
                if local_name.as_ref() == b"group" {
                    let group_name = get_attr(e, "name").unwrap_or_default();
                    let group_type = get_attr(e, "type").unwrap_or_else(|| "SelectAny".to_string());
                    let group = parse_group(reader, buf, group_name, group_type);
                    step.groups.push(group);
                    // parse_group consumes the End event
                    depth -= 1;
                }
            }
            Ok(Event::End(_)) => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    step
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Attempt to find and parse a FOMOD installer from the given directory.
///
/// Looks for `fomod/ModuleConfig.xml` (case-insensitive) inside `fomod_dir`.
/// Returns `Ok(None)` if no FOMOD configuration is found, or
/// `Ok(Some(installer))` on success.
pub fn parse_fomod(fomod_dir: &Path) -> Result<Option<FomodInstaller>> {
    // Locate the fomod subdirectory (case-insensitive)
    let fomod_subdir = find_case_insensitive(fomod_dir, "fomod");
    let fomod_subdir = match fomod_subdir {
        Some(p) => p,
        None => return Ok(None),
    };

    // Locate ModuleConfig.xml (case-insensitive)
    let config_path = find_case_insensitive(&fomod_subdir, "ModuleConfig.xml");
    let config_path = match config_path {
        Some(p) => p,
        None => return Ok(None),
    };

    let xml_content = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read FOMOD config: {}", config_path.display()))?;

    let installer = parse_fomod_xml(&xml_content)
        .with_context(|| format!("Failed to parse FOMOD config: {}", config_path.display()))?;

    Ok(Some(installer))
}

/// Determine default selections for each group in the installer.
///
/// Returns a map from group name to a list of selected option names.
/// Selection rules:
/// - `SelectAll` groups: all options selected.
/// - `SelectExactlyOne` / `SelectAtMostOne`: first `Required` or
///   `Recommended` option, or the first option if none qualify.
/// - `SelectAtLeastOne` / `SelectAny`: all `Required` and `Recommended`
///   options; falls back to the first option for `SelectAtLeastOne`.
pub fn get_default_selections(installer: &FomodInstaller) -> HashMap<String, Vec<String>> {
    let mut selections = HashMap::new();

    for step in &installer.steps {
        for group in &step.groups {
            let selected = default_selections_for_group(group);
            selections.insert(group.name.clone(), selected);
        }
    }

    selections
}

/// Collect all files that should be installed for the given user selections.
///
/// This always includes `required_files` from the installer. For each group
/// whose name appears in `selections`, the files from every listed option are
/// appended. Files are returned sorted by priority ascending (lowest first),
/// so higher-priority files overwrite lower-priority ones when extracted in
/// order.
pub fn get_files_for_selections(
    installer: &FomodInstaller,
    selections: &HashMap<String, Vec<String>>,
) -> Vec<FomodFile> {
    let mut files: Vec<FomodFile> = installer.required_files.clone();

    for step in &installer.steps {
        for group in &step.groups {
            if let Some(selected_names) = selections.get(&group.name) {
                for option in &group.options {
                    if selected_names.contains(&option.name) {
                        files.extend(option.files.clone());
                    }
                }
            }
        }
    }

    // Sort by priority so higher-priority files are deployed last (winning).
    files.sort_by_key(|f| f.priority);
    files
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Find a child entry in `parent` whose name matches `target` case-insensitively.
fn find_case_insensitive(parent: &Path, target: &str) -> Option<std::path::PathBuf> {
    // Fast path: try exact match first.
    let exact = parent.join(target);
    if exact.exists() {
        return Some(exact);
    }

    // Slow path: scan directory entries.
    let target_lower = target.to_lowercase();
    let entries = fs::read_dir(parent).ok()?;
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().to_lowercase() == target_lower {
            return Some(entry.path());
        }
    }

    None
}

/// Compute default selections for a single group.
fn default_selections_for_group(group: &FomodGroup) -> Vec<String> {
    match group.group_type.as_str() {
        "SelectAll" => group.options.iter().map(|o| o.name.clone()).collect(),

        "SelectExactlyOne" | "SelectAtMostOne" => {
            // Prefer Required, then Recommended, then first.
            let pick = group
                .options
                .iter()
                .find(|o| o.type_descriptor == "Required")
                .or_else(|| {
                    group
                        .options
                        .iter()
                        .find(|o| o.type_descriptor == "Recommended")
                })
                .or(group.options.first());
            match pick {
                Some(o) => vec![o.name.clone()],
                None => Vec::new(),
            }
        }

        "SelectAtLeastOne" | "SelectAny" => {
            let mut selected: Vec<String> = group
                .options
                .iter()
                .filter(|o| o.type_descriptor == "Required" || o.type_descriptor == "Recommended")
                .map(|o| o.name.clone())
                .collect();

            // For SelectAtLeastOne, ensure at least one is selected.
            if selected.is_empty() && group.group_type == "SelectAtLeastOne" {
                if let Some(first) = group.options.first() {
                    selected.push(first.name.clone());
                }
            }

            selected
        }

        _ => Vec::new(),
    }
}

/// Parse raw XML content into a [`FomodInstaller`].
fn parse_fomod_xml(xml: &str) -> Result<FomodInstaller> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut installer = FomodInstaller {
        module_name: String::new(),
        required_files: Vec::new(),
        steps: Vec::new(),
    };

    // Track whether we are inside certain parent elements.
    let mut in_required_files = false;

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let local_name = e.local_name();
                match local_name.as_ref() {
                    b"moduleName" => {
                        installer.module_name = read_text_content(&mut reader, &mut buf);
                    }
                    b"requiredInstallFiles" => {
                        in_required_files = true;
                    }
                    b"installStep" => {
                        let step_name = get_attr(e, "name").unwrap_or_default();
                        let step = parse_step(&mut reader, &mut buf, step_name);
                        installer.steps.push(step);
                    }
                    b"file" if in_required_files => {
                        installer.required_files.push(parse_file_element(e, false));
                    }
                    b"folder" if in_required_files => {
                        installer.required_files.push(parse_file_element(e, true));
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                let local_name = e.local_name();
                match local_name.as_ref() {
                    b"file" if in_required_files => {
                        installer.required_files.push(parse_file_element(e, false));
                    }
                    b"folder" if in_required_files => {
                        installer.required_files.push(parse_file_element(e, true));
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                if e.local_name().as_ref() == b"requiredInstallFiles" {
                    in_required_files = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "XML parse error at position {}: {:?}",
                    reader.error_position(),
                    e
                ));
            }
            _ => {}
        }
    }

    Ok(installer)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal FOMOD XML for testing.
    const SAMPLE_XML: &str = r#"
<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <moduleName>Test Mod</moduleName>
    <requiredInstallFiles>
        <folder source="core" destination="" priority="0" />
    </requiredInstallFiles>
    <installSteps order="Explicit">
        <installStep name="Choose Options">
            <optionalFileGroups order="Explicit">
                <group name="Textures" type="SelectExactlyOne">
                    <plugins order="Explicit">
                        <plugin name="High Res">
                            <description>High resolution textures</description>
                            <image path="fomod/images/highres.png" />
                            <files>
                                <folder source="textures/high" destination="textures" priority="0" />
                            </files>
                            <typeDescriptor>
                                <type name="Recommended" />
                            </typeDescriptor>
                        </plugin>
                        <plugin name="Low Res">
                            <description>Low resolution textures</description>
                            <files>
                                <folder source="textures/low" destination="textures" priority="0" />
                            </files>
                            <typeDescriptor>
                                <type name="Optional" />
                            </typeDescriptor>
                        </plugin>
                    </plugins>
                </group>
                <group name="Extras" type="SelectAny">
                    <plugins order="Explicit">
                        <plugin name="ENB Preset">
                            <description>Includes ENB preset</description>
                            <files>
                                <file source="optional/enb.ini" destination="enb.ini" priority="1" />
                            </files>
                            <typeDescriptor>
                                <type name="Optional" />
                            </typeDescriptor>
                        </plugin>
                    </plugins>
                </group>
            </optionalFileGroups>
        </installStep>
    </installSteps>
</config>
"#;

    #[test]
    fn parse_sample_xml() {
        let installer = parse_fomod_xml(SAMPLE_XML).expect("parse should succeed");

        assert_eq!(installer.module_name, "Test Mod");
        assert_eq!(installer.required_files.len(), 1);
        assert_eq!(installer.required_files[0].source, "core");
        assert!(installer.required_files[0].is_folder);

        assert_eq!(installer.steps.len(), 1);
        let step = &installer.steps[0];
        assert_eq!(step.name, "Choose Options");
        assert_eq!(step.groups.len(), 2);

        let textures = &step.groups[0];
        assert_eq!(textures.name, "Textures");
        assert_eq!(textures.group_type, "SelectExactlyOne");
        assert_eq!(textures.options.len(), 2);

        let high_res = &textures.options[0];
        assert_eq!(high_res.name, "High Res");
        assert_eq!(high_res.description, "High resolution textures");
        assert_eq!(high_res.image, Some("fomod/images/highres.png".to_string()));
        assert_eq!(high_res.type_descriptor, "Recommended");
        assert_eq!(high_res.files.len(), 1);
        assert!(high_res.files[0].is_folder);

        let extras = &step.groups[1];
        assert_eq!(extras.name, "Extras");
        assert_eq!(extras.group_type, "SelectAny");
        assert_eq!(extras.options.len(), 1);
        assert_eq!(extras.options[0].files[0].source, "optional/enb.ini");
        assert!(!extras.options[0].files[0].is_folder);
    }

    #[test]
    fn default_selections_picks_recommended() {
        let installer = parse_fomod_xml(SAMPLE_XML).unwrap();
        let selections = get_default_selections(&installer);

        // SelectExactlyOne should pick the Recommended option.
        let tex = selections.get("Textures").unwrap();
        assert_eq!(tex, &vec!["High Res".to_string()]);

        // SelectAny with no Required/Recommended should be empty.
        let extras = selections.get("Extras").unwrap();
        assert!(extras.is_empty());
    }

    #[test]
    fn get_files_includes_required_and_selected() {
        let installer = parse_fomod_xml(SAMPLE_XML).unwrap();
        let mut selections = HashMap::new();
        selections.insert("Textures".to_string(), vec!["Low Res".to_string()]);
        selections.insert("Extras".to_string(), vec!["ENB Preset".to_string()]);

        let files = get_files_for_selections(&installer, &selections);

        // Should contain: required folder + Low Res folder + ENB file
        assert_eq!(files.len(), 3);

        let sources: Vec<&str> = files.iter().map(|f| f.source.as_str()).collect();
        assert!(sources.contains(&"core"));
        assert!(sources.contains(&"textures/low"));
        assert!(sources.contains(&"optional/enb.ini"));
    }

    #[test]
    fn files_sorted_by_priority() {
        let installer = parse_fomod_xml(SAMPLE_XML).unwrap();
        let mut selections = HashMap::new();
        selections.insert("Textures".to_string(), vec!["High Res".to_string()]);
        selections.insert("Extras".to_string(), vec!["ENB Preset".to_string()]);

        let files = get_files_for_selections(&installer, &selections);
        let priorities: Vec<i32> = files.iter().map(|f| f.priority).collect();

        // Should be sorted ascending.
        for window in priorities.windows(2) {
            assert!(window[0] <= window[1]);
        }
    }

    #[test]
    fn parse_fomod_no_fomod_dir_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let result = parse_fomod(tmp.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_fomod_with_valid_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let fomod_dir = tmp.path().join("fomod");
        std::fs::create_dir_all(&fomod_dir).unwrap();
        std::fs::write(fomod_dir.join("ModuleConfig.xml"), SAMPLE_XML).unwrap();

        let result = parse_fomod(tmp.path()).unwrap();
        assert!(result.is_some());

        let installer = result.unwrap();
        assert_eq!(installer.module_name, "Test Mod");
    }

    #[test]
    fn select_all_group_selects_everything() {
        let group = FomodGroup {
            name: "All".into(),
            group_type: "SelectAll".into(),
            options: vec![
                FomodOption {
                    name: "A".into(),
                    description: String::new(),
                    image: None,
                    files: Vec::new(),
                    type_descriptor: "Optional".into(),
                },
                FomodOption {
                    name: "B".into(),
                    description: String::new(),
                    image: None,
                    files: Vec::new(),
                    type_descriptor: "Optional".into(),
                },
            ],
        };

        let selected = default_selections_for_group(&group);
        assert_eq!(selected, vec!["A".to_string(), "B".to_string()]);
    }
}
