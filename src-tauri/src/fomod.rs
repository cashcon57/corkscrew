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
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::RwLock;

use anyhow::{Context, Result};
use lru::LruCache;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// A single flag dependency used in visibility conditions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlagDependency {
    /// The flag name to check.
    pub flag: String,
    /// The expected value.
    pub value: String,
}

/// A game version dependency used in visibility conditions.
///
/// Corresponds to `<gameDependency version="x.x.x"/>` in FOMOD XML.
/// The condition is met when the detected game version >= the required version.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameDependency {
    /// The minimum required game version (e.g. "1.6.1170").
    pub version: String,
}

/// A file/plugin dependency used in conditions.
///
/// Corresponds to `<fileDependency file="SkyUI_SE.esp" state="Active"/>` in FOMOD XML.
/// Checks whether a file exists (or is active/inactive) in the game data directory.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileDependency {
    /// Relative path to check in the game data directory.
    pub file: String,
    /// Expected state: `"Active"`, `"Inactive"`, or `"Missing"`.
    pub state: String,
}

/// Evaluation context passed to condition evaluation functions.
///
/// Bundles all runtime state needed to evaluate FOMOD conditions so that
/// function signatures stay clean as we add more condition types.
pub struct FomodContext<'a> {
    /// Current condition flag state (accumulated across steps).
    pub flags: &'a HashMap<String, String>,
    /// Detected game version (e.g. `"1.6.1170"`), or `None` if unavailable.
    pub game_version: Option<&'a str>,
    /// Game data directory for file dependency checks, or `None` if unavailable.
    pub data_dir: Option<&'a Path>,
    /// Detected script extender version (e.g. `"2.2.6"`), or `None`.
    pub skse_version: Option<&'a str>,
}

/// A composite condition block with an operator (`And` / `Or`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConditionBlock {
    /// `"And"` or `"Or"` — how child conditions are combined.
    pub operator: String,
    /// Individual flag checks.
    pub flags: Vec<FlagDependency>,
    /// Game version checks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub game_dependencies: Vec<GameDependency>,
    /// File/plugin existence checks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_dependencies: Vec<FileDependency>,
    /// Nested composite condition blocks (recursive tree).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ConditionBlock>,
}

impl ConditionBlock {
    /// Evaluate this condition block against the provided context.
    ///
    /// When a context field is `None`, the corresponding conditions are assumed
    /// met (permissive fallback for contexts where data is unavailable).
    pub fn evaluate(&self, ctx: &FomodContext) -> bool {
        let all_empty = self.flags.is_empty()
            && self.game_dependencies.is_empty()
            && self.file_dependencies.is_empty()
            && self.children.is_empty();
        if all_empty {
            return true;
        }

        let check_flag = |dep: &FlagDependency| ctx.flags.get(&dep.flag) == Some(&dep.value);
        let check_game = |dep: &GameDependency| {
            ctx.game_version
                .map_or(true, |v| version_gte(v, &dep.version))
        };
        let check_file = |dep: &FileDependency| evaluate_file_dependency(dep, ctx.data_dir);

        match self.operator.as_str() {
            "Or" => {
                self.flags.iter().any(|d| check_flag(d))
                    || self.game_dependencies.iter().any(|d| check_game(d))
                    || self.file_dependencies.iter().any(|d| check_file(d))
                    || self.children.iter().any(|c| c.evaluate(ctx))
            }
            // Default to And
            _ => {
                self.flags.iter().all(|d| check_flag(d))
                    && self.game_dependencies.iter().all(|d| check_game(d))
                    && self.file_dependencies.iter().all(|d| check_file(d))
                    && self.children.iter().all(|c| c.evaluate(ctx))
            }
        }
    }
}

/// Evaluate a file dependency condition against the game data directory.
///
/// If `data_dir` is `None`, returns `true` (permissive fallback).
fn evaluate_file_dependency(dep: &FileDependency, data_dir: Option<&Path>) -> bool {
    let data_dir = match data_dir {
        Some(d) => d,
        None => return true, // Permissive when we don't have data dir
    };

    // Normalize Windows backslashes to platform separators
    let rel_path = dep.file.replace('\\', "/");

    // Reject path traversal attempts (e.g. "../../../etc/passwd")
    if !crate::staging::is_safe_relative_path(&rel_path) {
        log::warn!("FOMOD file dependency has unsafe path, skipping: {}", rel_path);
        return false;
    }

    let file_path = data_dir.join(&rel_path);
    let exists = file_path.exists();

    match dep.state.as_str() {
        "Active" | "active" => exists, // Treat "Active" as "file is present"
        "Inactive" | "inactive" => exists, // File present but not active — we treat as present
        "Missing" | "missing" => !exists,
        _ => exists, // Default: check existence
    }
}

/// Compare two dotted version strings numerically.
/// Returns `true` if `actual >= required`.
fn version_gte(actual: &str, required: &str) -> bool {
    // Split on '.' and parse each component.  An unparseable component
    // (e.g. "x" in "1.6.x") is treated as a wildcard that satisfies any
    // required value — this prevents unknown AE version strings like
    // "1.6.x (Anniversary Edition, ~35 MB)" from failing checks like
    // `>= 1.6.640`.
    let parts_actual: Vec<&str> = actual.split('.').collect();
    let parts_required: Vec<&str> = required.split('.').collect();
    let len = parts_actual.len().max(parts_required.len());
    for i in 0..len {
        let a_str = parts_actual.get(i).copied().unwrap_or("0");
        let r_str = parts_required.get(i).copied().unwrap_or("0");
        let av = a_str.parse::<u64>();
        let rv = r_str.parse::<u64>().unwrap_or(0);
        match av {
            Ok(av) => match av.cmp(&rv) {
                std::cmp::Ordering::Greater => return true,
                std::cmp::Ordering::Less => return false,
                std::cmp::Ordering::Equal => continue,
            },
            // Unparseable component (wildcard) — treat as always >= required
            Err(_) => return true,
        }
    }
    true // Equal versions
}

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

/// A conditional type pattern: if the condition is met, the option type changes.
///
/// Corresponds to `<pattern>` inside `<dependencyType><patterns>`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConditionalTypePattern {
    /// The condition that must be met for this type to apply.
    pub condition: ConditionBlock,
    /// The option type to use when the condition is met (e.g. "Required", "NotUsable").
    pub option_type: String,
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
    /// Static type descriptor (fallback when no conditional patterns match).
    /// Common values: `"Optional"`, `"Required"`, `"Recommended"`,
    /// `"NotUsable"`, `"CouldBeUsable"`.
    pub type_descriptor: String,
    /// Conditional type patterns — evaluated in order, first match wins.
    /// If no pattern matches, `type_descriptor` is used as the fallback.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditional_type_patterns: Vec<ConditionalTypePattern>,
    /// Condition flags set when this option is selected.
    /// Maps flag name to flag value.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub condition_flags: HashMap<String, String>,
}

impl FomodOption {
    /// Resolve the effective type descriptor, evaluating conditional patterns.
    ///
    /// Returns the `option_type` from the first matching pattern, or
    /// `self.type_descriptor` if none match.
    pub fn effective_type(&self, ctx: &FomodContext) -> &str {
        for pattern in &self.conditional_type_patterns {
            if pattern.condition.evaluate(ctx) {
                return &pattern.option_type;
            }
        }
        &self.type_descriptor
    }
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
    /// Optional visibility condition — step is shown only when this evaluates
    /// to `true` (or when `None`, meaning always visible).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<ConditionBlock>,
}

/// A conditional file install pattern — files installed when a condition is met,
/// independent of user selections.
///
/// Corresponds to `<pattern>` inside `<conditionalFileInstalls><patterns>`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConditionalFileInstall {
    /// The condition that must be met for these files to be installed.
    pub condition: ConditionBlock,
    /// Files to install when the condition is met.
    pub files: Vec<FomodFile>,
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
    /// Top-level module dependencies / prerequisites.
    /// If present and not met, the installer should warn the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_dependencies: Option<ConditionBlock>,
    /// Conditional file installs — file sets auto-installed when conditions are met.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditional_file_installs: Vec<ConditionalFileInstall>,
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
            // Use unescape_value to decode XML entities (&quot;, &amp;, etc.)
            return attr.unescape_value().ok().map(|s| s.into_owned());
        }
    }
    None
}

/// Strip a leading `Data/` (case-insensitive) prefix from a FOMOD destination
/// path. Many Windows-authored FOMOD XMLs include `destination="Data\meshes"`
/// but we deploy INTO the Data directory, so the prefix would create a nested
/// `Data/Data/meshes/` layout.
fn strip_data_prefix(path: &str) -> String {
    let lower = path.to_lowercase();
    if lower == "data" || lower == "data/" {
        return String::new();
    }
    if lower.starts_with("data/") {
        return path[5..].to_string();
    }
    path.to_string()
}

/// Parse a `<file>` or `<folder>` element into a [`FomodFile`].
///
/// Normalises Windows backslash separators to forward slashes and strips any
/// leading `Data/` prefix from destinations so that files deploy correctly on
/// macOS / Linux.
fn parse_file_element(tag: &quick_xml::events::BytesStart<'_>, is_folder: bool) -> FomodFile {
    // Normalise backslash → forward-slash (FOMOD XMLs from Windows use backslashes).
    let source = get_attr(tag, "source").unwrap_or_default().replace('\\', "/");
    let destination = get_attr(tag, "destination").unwrap_or_default().replace('\\', "/");
    // Strip "Data/" prefix from destination — we already deploy into Data/.
    let destination = strip_data_prefix(&destination);
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

/// Parse a `<visible>`, `<dependencies>`, or `<moduleDependencies>` block into
/// a [`ConditionBlock`].
///
/// Handles flat and nested structures:
/// ```xml
/// <visible>
///   <dependencies operator="And">
///     <flagDependency flag="name" value="val"/>
///     <fileDependency file="SkyUI_SE.esp" state="Active"/>
///     <gameDependency version="1.6.0"/>
///   </dependencies>
/// </visible>
/// ```
///
/// Also handles nested composite conditions:
/// ```xml
/// <dependencies operator="Or">
///   <dependencies operator="And">
///     <flagDependency flag="A" value="1"/>
///     <gameDependency version="1.6.0"/>
///   </dependencies>
///   <flagDependency flag="B" value="1"/>
/// </dependencies>
/// ```
///
/// `fommDependency` and script extender dependencies (`foseDependency`,
/// `nvseDependency`, `skseDependency`, `f4seDependency`) are parsed but
/// always evaluate as met (permissive).
fn parse_condition_block(
    reader: &mut Reader<&[u8]>,
    buf: &mut Vec<u8>,
    end_tag: &[u8],
) -> Option<ConditionBlock> {
    let mut block: Option<ConditionBlock> = None;
    let mut depth = 1u32;
    loop {
        buf.clear();
        match reader.read_event_into(buf) {
            Ok(Event::Start(ref e)) => {
                depth += 1;
                let local = e.local_name();
                if local.as_ref() == b"dependencies" {
                    if block.is_none() {
                        // First <dependencies> — this IS the block
                        let op = get_attr(e, "operator").unwrap_or_else(|| "And".to_string());
                        block = Some(ConditionBlock {
                            operator: op,
                            flags: Vec::new(),
                            game_dependencies: Vec::new(),
                            file_dependencies: Vec::new(),
                            children: Vec::new(),
                        });
                    } else {
                        // Nested <dependencies> — recurse to create a child block
                        let op = get_attr(e, "operator").unwrap_or_else(|| "And".to_string());
                        let child =
                            parse_nested_condition_block(reader, buf, &op);
                        if let Some(ref mut b) = block {
                            b.children.push(child);
                        }
                        // parse_nested consumed the </dependencies> end tag
                        depth -= 1;
                    }
                }
            }
            Ok(Event::Empty(ref e)) => {
                let local = e.local_name();
                match local.as_ref() {
                    b"flagDependency" => {
                        if let (Some(flag), Some(value)) =
                            (get_attr(e, "flag"), get_attr(e, "value"))
                        {
                            if let Some(ref mut b) = block {
                                b.flags.push(FlagDependency { flag, value });
                            }
                        }
                    }
                    b"gameDependency" => {
                        if let Some(version) = get_attr(e, "version") {
                            if let Some(ref mut b) = block {
                                b.game_dependencies.push(GameDependency { version });
                            }
                        }
                    }
                    b"fileDependency" => {
                        if let (Some(file), Some(state)) =
                            (get_attr(e, "file"), get_attr(e, "state"))
                        {
                            if let Some(ref mut b) = block {
                                b.file_dependencies.push(FileDependency { file, state });
                            }
                        }
                    }
                    // Mod manager version — always pass (any modern manager suffices)
                    b"fommDependency" => { /* intentionally ignored — always met */ }
                    // Script extender version deps — parsed but treated as met
                    // (we don't block installs based on SE version)
                    b"foseDependency" | b"nvseDependency" | b"skseDependency"
                    | b"f4seDependency" => { /* permissive — always met */ }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                depth -= 1;
                if depth == 0 || e.local_name().as_ref() == end_tag {
                    break;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    block
}

/// Parse a nested `<dependencies>` block (already inside a parent block).
///
/// Called when we encounter a `<dependencies>` start tag inside another
/// `<dependencies>`. Returns a complete child `ConditionBlock`.
fn parse_nested_condition_block(
    reader: &mut Reader<&[u8]>,
    buf: &mut Vec<u8>,
    operator: &str,
) -> ConditionBlock {
    let mut block = ConditionBlock {
        operator: operator.to_string(),
        flags: Vec::new(),
        game_dependencies: Vec::new(),
        file_dependencies: Vec::new(),
        children: Vec::new(),
    };
    let mut depth = 1u32;
    loop {
        buf.clear();
        match reader.read_event_into(buf) {
            Ok(Event::Start(ref e)) => {
                depth += 1;
                let local = e.local_name();
                if local.as_ref() == b"dependencies" {
                    let op = get_attr(e, "operator").unwrap_or_else(|| "And".to_string());
                    let child = parse_nested_condition_block(reader, buf, &op);
                    block.children.push(child);
                    depth -= 1;
                }
            }
            Ok(Event::Empty(ref e)) => {
                let local = e.local_name();
                match local.as_ref() {
                    b"flagDependency" => {
                        if let (Some(flag), Some(value)) =
                            (get_attr(e, "flag"), get_attr(e, "value"))
                        {
                            block.flags.push(FlagDependency { flag, value });
                        }
                    }
                    b"gameDependency" => {
                        if let Some(version) = get_attr(e, "version") {
                            block.game_dependencies.push(GameDependency { version });
                        }
                    }
                    b"fileDependency" => {
                        if let (Some(file), Some(state)) =
                            (get_attr(e, "file"), get_attr(e, "state"))
                        {
                            block.file_dependencies.push(FileDependency { file, state });
                        }
                    }
                    b"fommDependency" | b"foseDependency" | b"nvseDependency"
                    | b"skseDependency" | b"f4seDependency" => { /* always met */ }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                depth -= 1;
                if depth == 0 || e.local_name().as_ref() == b"dependencies" {
                    break;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    block
}

/// Parse a `<conditionFlags>` block into a flag name→value map.
///
/// Handles the structure:
/// ```xml
/// <conditionFlags>
///   <flag name="someFlag">On</flag>
/// </conditionFlags>
/// ```
fn parse_condition_flags(reader: &mut Reader<&[u8]>, buf: &mut Vec<u8>) -> HashMap<String, String> {
    let mut flags = HashMap::new();
    let mut depth = 1u32;
    loop {
        buf.clear();
        match reader.read_event_into(buf) {
            Ok(Event::Start(ref e)) => {
                depth += 1;
                let local = e.local_name();
                if local.as_ref() == b"flag" {
                    let flag_name = get_attr(e, "name").unwrap_or_default();
                    let flag_value = read_text_content(reader, buf);
                    flags.insert(flag_name, flag_value);
                    // read_text_content consumes the End event
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
    flags
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
        conditional_type_patterns: Vec::new(),
        condition_flags: HashMap::new(),
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
                    b"conditionFlags" => {
                        option.condition_flags = parse_condition_flags(reader, buf);
                        // parse_condition_flags consumes the End event
                        depth -= 1;
                    }
                    b"dependencyType" => {
                        let (default_type, patterns) =
                            parse_dependency_type(reader, buf);
                        option.type_descriptor = default_type;
                        option.conditional_type_patterns = patterns;
                        // parse_dependency_type consumes the End event
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

/// Parse a `<dependencyType>` block inside `<typeDescriptor>`.
///
/// Returns `(default_type, patterns)` where `default_type` is the fallback
/// type name and `patterns` is a list of conditional type overrides.
///
/// ```xml
/// <dependencyType>
///   <defaultType name="Optional"/>
///   <patterns>
///     <pattern>
///       <dependencies operator="And">
///         <fileDependency file="SkyUI_SE.esp" state="Active"/>
///       </dependencies>
///       <type name="Recommended"/>
///     </pattern>
///   </patterns>
/// </dependencyType>
/// ```
fn parse_dependency_type(
    reader: &mut Reader<&[u8]>,
    buf: &mut Vec<u8>,
) -> (String, Vec<ConditionalTypePattern>) {
    let mut default_type = "Optional".to_string();
    let mut patterns = Vec::new();
    let mut depth = 1u32;
    let mut in_pattern = false;
    let mut current_condition: Option<ConditionBlock> = None;
    let mut current_type: Option<String> = None;

    loop {
        buf.clear();
        match reader.read_event_into(buf) {
            Ok(Event::Start(ref e)) => {
                depth += 1;
                let local = e.local_name();
                match local.as_ref() {
                    b"pattern" => {
                        in_pattern = true;
                        current_condition = None;
                        current_type = None;
                    }
                    b"dependencies" if in_pattern => {
                        let op = get_attr(e, "operator").unwrap_or_else(|| "And".to_string());
                        current_condition = Some(parse_nested_condition_block(reader, buf, &op));
                        depth -= 1; // parse_nested consumed end tag
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                let local = e.local_name();
                match local.as_ref() {
                    b"defaultType" => {
                        if let Some(name) = get_attr(e, "name") {
                            default_type = name;
                        }
                    }
                    b"type" if in_pattern => {
                        current_type = get_attr(e, "name");
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                depth -= 1;
                if e.local_name().as_ref() == b"pattern" {
                    if let (Some(cond), Some(opt_type)) =
                        (current_condition.take(), current_type.take())
                    {
                        patterns.push(ConditionalTypePattern {
                            condition: cond,
                            option_type: opt_type,
                        });
                    }
                    in_pattern = false;
                }
                if depth == 0 {
                    break;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    (default_type, patterns)
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
        visible: None,
    };

    let mut depth = 1u32;
    loop {
        buf.clear();
        match reader.read_event_into(buf) {
            Ok(Event::Start(ref e)) => {
                depth += 1;
                let local_name = e.local_name();
                match local_name.as_ref() {
                    b"group" => {
                        let group_name = get_attr(e, "name").unwrap_or_default();
                        let group_type =
                            get_attr(e, "type").unwrap_or_else(|| "SelectAny".to_string());
                        let group = parse_group(reader, buf, group_name, group_type);
                        step.groups.push(group);
                        // parse_group consumes the End event
                        depth -= 1;
                    }
                    b"visible" => {
                        step.visible = parse_condition_block(reader, buf, b"visible");
                        // parse_condition_block consumes the End event
                        depth -= 1;
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

    let xml_content = read_xml_any_encoding(&config_path)
        .with_context(|| format!("Failed to read FOMOD config: {}", config_path.display()))?;

    // Strip the encoding="..." declaration — many FOMOD XMLs declare
    // "Windows-1252" or "utf-16" but are actually UTF-8 after our BOM-based
    // decoding. The wrong declaration confuses quick_xml.
    let xml_content = strip_xml_encoding_declaration(&xml_content);

    let installer = parse_fomod_xml(&xml_content)
        .with_context(|| format!("Failed to parse FOMOD config: {}", config_path.display()))?;

    Ok(Some(installer))
}

/// Default capacity for the FOMOD LRU cache (covers a typical modlist).
pub const FOMOD_CACHE_CAPACITY: usize = 50;

/// Create a new, empty FOMOD LRU cache with the default capacity.
pub fn new_fomod_cache() -> RwLock<LruCache<String, FomodInstaller>> {
    RwLock::new(LruCache::new(
        NonZeroUsize::new(FOMOD_CACHE_CAPACITY).unwrap(),
    ))
}

/// Parse a FOMOD installer with LRU caching.
///
/// If a cached result exists for `cache_key` (typically the archive's
/// SHA-256 hash), it is returned immediately. Otherwise the FOMOD is
/// parsed from disk and stored in the cache.
///
/// The `cache` is passed in from [`AppState`] rather than being a static.
pub fn parse_fomod_cached(
    cache: &RwLock<LruCache<String, FomodInstaller>>,
    cache_key: &str,
    fomod_dir: &Path,
) -> Result<Option<FomodInstaller>> {
    // Check cache (write lock needed because `get` updates LRU order)
    {
        let mut cache_guard = cache
            .write()
            .map_err(|_| anyhow::anyhow!("FOMOD cache lock poisoned"))?;
        if let Some(cached) = cache_guard.get(cache_key) {
            log::debug!("FOMOD cache hit for key '{}'", cache_key);
            return Ok(Some(cached.clone()));
        }
    }

    // Cache miss — parse from disk
    let installer = parse_fomod(fomod_dir)?;

    // Store in cache if parsing succeeded
    if let Some(ref inst) = installer {
        let mut cache_guard = cache
            .write()
            .map_err(|_| anyhow::anyhow!("FOMOD cache lock poisoned"))?;
        cache_guard.put(cache_key.to_string(), inst.clone());
        log::debug!("FOMOD cache miss — stored key '{}'", cache_key);
    }

    Ok(installer)
}

/// Determine default selections for each group in the installer.
///
/// Returns a map from group name to a list of selected option names.
/// Tracks condition flags across steps so that steps with `<visible>`
/// conditions are skipped when their dependencies are not met.
///
/// Selection rules:
/// Resolve relative image paths in every option to absolute filesystem paths
/// rooted at `staging_dir`.  This lets the frontend convert them with
/// `convertFileSrc()` for the `asset:` protocol.
pub fn resolve_image_paths(installer: &mut FomodInstaller, staging_dir: &Path) {
    for step in &mut installer.steps {
        for group in &mut step.groups {
            for option in &mut group.options {
                if let Some(ref rel) = option.image {
                    let abs = staging_dir.join(rel);
                    if abs.exists() {
                        option.image = Some(abs.to_string_lossy().into_owned());
                    } else {
                        // Try case-insensitive lookup — FOMOD paths are often
                        // authored on Windows with mismatched casing.
                        if let Some(found) = resolve_case_insensitive(staging_dir, rel) {
                            option.image = Some(found.to_string_lossy().into_owned());
                        }
                        // else leave relative — image just won't render
                    }
                }
            }
        }
    }
}

/// Walk path components case-insensitively from `base`.
fn resolve_case_insensitive(base: &Path, relative: &str) -> Option<std::path::PathBuf> {
    let mut current = base.to_path_buf();
    for component in relative.replace('\\', "/").split('/') {
        if component.is_empty() {
            continue;
        }
        let entries = std::fs::read_dir(&current).ok()?;
        let mut found = false;
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().eq_ignore_ascii_case(component) {
                current = entry.path();
                found = true;
                break;
            }
        }
        if !found {
            return None;
        }
    }
    Some(current)
}

/// - `SelectAll` groups: all options selected.
/// - `SelectExactlyOne` / `SelectAtMostOne`: first `Required` or
///   `Recommended` option, or the first option if none qualify.
/// - `SelectAtLeastOne` / `SelectAny`: all `Required` and `Recommended`
///   options; falls back to the first option for `SelectAtLeastOne`.
///
/// When conditional type descriptors are present (`dependencyType`), the
/// effective type is resolved using the provided context so that options
/// may change from Optional to Required (or NotUsable) based on installed
/// files, game version, or flags.
pub fn get_default_selections(
    installer: &FomodInstaller,
    game_version: Option<&str>,
    data_dir: Option<&Path>,
) -> HashMap<String, Vec<String>> {
    let mut selections = HashMap::new();
    let mut condition_flags: HashMap<String, String> = HashMap::new();

    for step in &installer.steps {
        // Create context scoped so the borrow is dropped before we mutate condition_flags.
        let visible = {
            let ctx = FomodContext {
                flags: &condition_flags,
                game_version,
                data_dir,
                skse_version: None,
            };
            step_is_visible(step, &ctx)
        };
        if !visible {
            continue;
        }

        for group in &step.groups {
            let selected = {
                let ctx = FomodContext {
                    flags: &condition_flags,
                    game_version,
                    data_dir,
                    skse_version: None,
                };
                default_selections_for_group(group, &ctx)
            };
            // Update condition flags from selected options.
            for option in &group.options {
                if selected.contains(&option.name) {
                    for (k, v) in &option.condition_flags {
                        condition_flags.insert(k.clone(), v.clone());
                    }
                }
            }
            selections.insert(group.name.clone(), selected);
        }
    }

    selections
}

/// Collect all files that should be installed for the given user selections.
///
/// This always includes `required_files` from the installer. For each group
/// whose name appears in `selections`, the files from every listed option are
/// appended. Steps with unmet `<visible>` conditions are skipped. Condition
/// flags are tracked across steps to evaluate visibility.
///
/// After processing user selections, `conditionalFileInstalls` patterns are
/// evaluated — file sets whose conditions are met are automatically included.
///
/// Files are returned sorted by priority ascending (lowest first), so
/// higher-priority files overwrite lower-priority ones when extracted in order.
pub fn get_files_for_selections(
    installer: &FomodInstaller,
    selections: &HashMap<String, Vec<String>>,
    game_version: Option<&str>,
    data_dir: Option<&Path>,
) -> Vec<FomodFile> {
    let mut files: Vec<FomodFile> = installer.required_files.clone();
    let mut condition_flags: HashMap<String, String> = HashMap::new();

    for step in &installer.steps {
        let visible = {
            let ctx = FomodContext {
                flags: &condition_flags,
                game_version,
                data_dir,
                skse_version: None,
            };
            step_is_visible(step, &ctx)
        };
        if !visible {
            continue;
        }

        for group in &step.groups {
            // Try exact match first, then case-insensitive lookup.
            let selected_names = selections.get(&group.name)
                .or_else(|| {
                    let lower = group.name.to_lowercase();
                    selections.iter()
                        .find(|(k, _)| k.to_lowercase() == lower)
                        .map(|(_, v)| v)
                });

            if let Some(names) = selected_names {
                for option in &group.options {
                    if names.contains(&option.name) {
                        files.extend(option.files.clone());
                        for (k, v) in &option.condition_flags {
                            condition_flags.insert(k.clone(), v.clone());
                        }
                    }
                }
            } else {
                // No match in selections — apply defaults for this group so
                // Required/Recommended options are not silently dropped.
                let ctx = FomodContext {
                    flags: &condition_flags,
                    game_version,
                    data_dir,
                    skse_version: None,
                };
                let defaults = default_selections_for_group(group, &ctx);
                if !defaults.is_empty() {
                    log::info!(
                        "FOMOD: no selection for group '{}' — applying defaults: [{}]",
                        group.name,
                        defaults.join(", ")
                    );
                }
                for option in &group.options {
                    if defaults.contains(&option.name) {
                        files.extend(option.files.clone());
                        for (k, v) in &option.condition_flags {
                            condition_flags.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
        }
    }

    // Evaluate conditionalFileInstalls — auto-install file sets when conditions are met.
    if !installer.conditional_file_installs.is_empty() {
        let ctx = FomodContext {
            flags: &condition_flags,
            game_version,
            data_dir,
            skse_version: None,
        };
        for cfi in &installer.conditional_file_installs {
            if cfi.condition.evaluate(&ctx) {
                log::info!(
                    "conditionalFileInstall matched — adding {} files",
                    cfi.files.len()
                );
                files.extend(cfi.files.clone());
            }
        }
    }

    // Sort by priority so higher-priority files are deployed last (winning).
    files.sort_by_key(|f| f.priority);
    files
}

/// Check module-level prerequisites and return any unmet dependency warnings.
///
/// Returns `None` if prerequisites are met or absent, or `Some(message)` if
/// the user should be warned.
pub fn check_module_dependencies(
    installer: &FomodInstaller,
    ctx: &FomodContext,
) -> Option<String> {
    if let Some(ref deps) = installer.module_dependencies {
        if !deps.evaluate(ctx) {
            let mut details = Vec::new();
            for fd in &deps.file_dependencies {
                details.push(format!("file '{}' state={}", fd.file, fd.state));
            }
            for gd in &deps.game_dependencies {
                details.push(format!("game version >= {}", gd.version));
            }
            let msg = if details.is_empty() {
                "Module prerequisites not met".to_string()
            } else {
                format!("Module prerequisites not met: {}", details.join(", "))
            };
            return Some(msg);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Remove the `encoding="..."` attribute from an XML declaration header.
///
/// After BOM-based decoding we always have a valid UTF-8 string, but the
/// original XML may still declare `encoding="Windows-1252"` or `encoding="utf-16"`
/// which confuses the parser. Stripping it lets quick_xml default to UTF-8.
fn strip_xml_encoding_declaration(xml: &str) -> String {
    // Only look at the first 200 bytes — the XML declaration is always at the start.
    let search_area = &xml[..xml.len().min(200)];
    let header_start = match search_area.find("<?xml") {
        Some(pos) => pos,
        None => return xml.to_string(),
    };
    let header_end = match xml[header_start..].find("?>") {
        Some(pos) => header_start + pos + 2,
        None => return xml.to_string(),
    };
    let header = &xml[header_start..header_end];

    // Find the encoding attribute (case-insensitive key, any quote style).
    let header_lower = header.to_lowercase();
    let enc_offset = match header_lower.find("encoding") {
        Some(pos) => pos,
        None => return xml.to_string(), // No encoding attribute — nothing to strip.
    };

    // Walk forward from "encoding" to find = then the quoted value.
    let after_key = &header[enc_offset..];
    let eq_pos = match after_key.find('=') {
        Some(pos) => pos,
        None => return xml.to_string(),
    };
    let after_eq = after_key[eq_pos + 1..].trim_start();
    let quote = match after_eq.bytes().next() {
        Some(b'"') | Some(b'\'') => after_eq.as_bytes()[0] as char,
        _ => return xml.to_string(),
    };
    let value_start = 1; // skip opening quote
    let value_end = match after_eq[value_start..].find(quote) {
        Some(pos) => value_start + pos,
        None => return xml.to_string(),
    };

    // Compute byte offsets within the original XML.
    let attr_start_in_xml = header_start + enc_offset;
    let consumed_in_after_key = eq_pos + 1 + (after_key[eq_pos + 1..].len() - after_eq.len()) + value_end + 1;
    let attr_end_in_xml = attr_start_in_xml + consumed_in_after_key;

    // Rebuild XML without the encoding attribute (also trim any trailing space).
    let mut result = String::with_capacity(xml.len());
    result.push_str(&xml[..attr_start_in_xml]);
    let rest = &xml[attr_end_in_xml..];
    // Avoid double spaces where the attribute was.
    result.push_str(rest.strip_prefix(' ').unwrap_or(rest));
    result
}

/// Read an XML file that may be encoded as UTF-8, UTF-8 with BOM, or UTF-16
/// (LE/BE). Many FOMOD configs from Windows mod authors are saved as UTF-16.
fn read_xml_any_encoding(path: &Path) -> Result<String> {
    let raw = fs::read(path)?;

    if raw.len() < 2 {
        // Too small to have a BOM — treat as UTF-8
        return Ok(String::from_utf8_lossy(&raw).into_owned());
    }

    // UTF-16 LE BOM: FF FE
    if raw[0] == 0xFF && raw[1] == 0xFE {
        let u16_iter = raw[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
        let decoded: String = char::decode_utf16(u16_iter)
            .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
        log::info!("FOMOD config decoded from UTF-16 LE: {}", path.display());
        return Ok(decoded);
    }

    // UTF-16 BE BOM: FE FF
    if raw[0] == 0xFE && raw[1] == 0xFF {
        let u16_iter = raw[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]));
        let decoded: String = char::decode_utf16(u16_iter)
            .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
        log::info!("FOMOD config decoded from UTF-16 BE: {}", path.display());
        return Ok(decoded);
    }

    // UTF-8 BOM: EF BB BF — strip it
    if raw.len() >= 3 && raw[0] == 0xEF && raw[1] == 0xBB && raw[2] == 0xBF {
        return Ok(String::from_utf8_lossy(&raw[3..]).into_owned());
    }

    // No BOM — try UTF-8 first, then fall back to lossy
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

/// Find a child entry in `parent` whose name matches `target` case-insensitively.
pub(crate) fn find_case_insensitive(parent: &Path, target: &str) -> Option<std::path::PathBuf> {
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

/// Resolve a multi-component relative path against `base`, matching each
/// component case-insensitively. Returns `None` if any component fails to
/// match. This is essential on case-sensitive filesystems (Linux / SteamOS)
/// where FOMOD source paths like `Textures/High` may not match `textures/high`.
pub(crate) fn resolve_path_case_insensitive(base: &Path, rel: &str) -> Option<std::path::PathBuf> {
    let mut current = base.to_path_buf();
    for component in rel.split('/').filter(|c| !c.is_empty()) {
        current = find_case_insensitive(&current, component)?;
    }
    Some(current)
}

/// Check whether a step's visibility condition is met.
/// Returns `true` if the step has no condition or if the condition evaluates to true.
fn step_is_visible(step: &FomodStep, ctx: &FomodContext) -> bool {
    match &step.visible {
        None => true,
        Some(cond) => cond.evaluate(ctx),
    }
}

/// Compute default selections for a single group, using conditional type
/// descriptors when available.
fn default_selections_for_group(group: &FomodGroup, ctx: &FomodContext) -> Vec<String> {
    match group.group_type.as_str() {
        "SelectAll" => group
            .options
            .iter()
            .filter(|o| o.effective_type(ctx) != "NotUsable")
            .map(|o| o.name.clone())
            .collect(),

        "SelectExactlyOne" | "SelectAtMostOne" => {
            // Prefer Required, then Recommended, then first usable.
            let pick = group
                .options
                .iter()
                .find(|o| o.effective_type(ctx) == "Required")
                .or_else(|| {
                    group
                        .options
                        .iter()
                        .find(|o| o.effective_type(ctx) == "Recommended")
                })
                .or_else(|| {
                    group
                        .options
                        .iter()
                        .find(|o| o.effective_type(ctx) != "NotUsable")
                });
            match pick {
                Some(o) => vec![o.name.clone()],
                None => Vec::new(),
            }
        }

        "SelectAtLeastOne" | "SelectAny" => {
            let mut selected: Vec<String> = group
                .options
                .iter()
                .filter(|o| {
                    let t = o.effective_type(ctx);
                    t == "Required" || t == "Recommended"
                })
                .map(|o| o.name.clone())
                .collect();

            // For SelectAtLeastOne, ensure at least one is selected.
            if selected.is_empty() && group.group_type == "SelectAtLeastOne" {
                if let Some(first) = group
                    .options
                    .iter()
                    .find(|o| o.effective_type(ctx) != "NotUsable")
                {
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
        module_dependencies: None,
        conditional_file_installs: Vec::new(),
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
                    b"moduleDependencies" => {
                        // Top-level prerequisites — parse directly as a condition block.
                        let op = get_attr(e, "operator").unwrap_or_else(|| "And".to_string());
                        let block =
                            parse_nested_condition_block(&mut reader, &mut buf, &op);
                        installer.module_dependencies = Some(block);
                    }
                    b"conditionalFileInstalls" => {
                        installer.conditional_file_installs =
                            parse_conditional_file_installs(&mut reader, &mut buf);
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

/// Parse a `<conditionalFileInstalls>` block.
///
/// ```xml
/// <conditionalFileInstalls>
///   <patterns>
///     <pattern>
///       <dependencies operator="And">
///         <flagDependency flag="X" value="Y"/>
///       </dependencies>
///       <files>
///         <folder source="compat/skyui" destination="" priority="0"/>
///       </files>
///     </pattern>
///   </patterns>
/// </conditionalFileInstalls>
/// ```
fn parse_conditional_file_installs(
    reader: &mut Reader<&[u8]>,
    buf: &mut Vec<u8>,
) -> Vec<ConditionalFileInstall> {
    let mut installs = Vec::new();
    let mut depth = 1u32;
    let mut in_pattern = false;
    let mut current_condition: Option<ConditionBlock> = None;
    let mut current_files: Vec<FomodFile> = Vec::new();

    loop {
        buf.clear();
        match reader.read_event_into(buf) {
            Ok(Event::Start(ref e)) => {
                depth += 1;
                let local = e.local_name();
                match local.as_ref() {
                    b"pattern" => {
                        in_pattern = true;
                        current_condition = None;
                        current_files = Vec::new();
                    }
                    b"dependencies" if in_pattern => {
                        let op = get_attr(e, "operator").unwrap_or_else(|| "And".to_string());
                        current_condition =
                            Some(parse_nested_condition_block(reader, buf, &op));
                        depth -= 1; // parse_nested consumed end tag
                    }
                    b"files" if in_pattern => {
                        current_files = parse_files_block(reader, buf);
                        depth -= 1; // parse_files_block consumed end tag
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                depth -= 1;
                if e.local_name().as_ref() == b"pattern" && in_pattern {
                    if let Some(cond) = current_condition.take() {
                        installs.push(ConditionalFileInstall {
                            condition: cond,
                            files: std::mem::take(&mut current_files),
                        });
                    }
                    in_pattern = false;
                }
                if depth == 0 {
                    break;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    installs
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
        let selections = get_default_selections(&installer, None, None);

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

        let files = get_files_for_selections(&installer, &selections, None, None);

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

        let files = get_files_for_selections(&installer, &selections, None, None);
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

    /// Helper to create a default FomodContext for tests.
    fn test_ctx(flags: &HashMap<String, String>) -> FomodContext {
        FomodContext {
            flags,
            game_version: None,
            data_dir: None,
            skse_version: None,
        }
    }

    fn test_ctx_with_game<'a>(
        flags: &'a HashMap<String, String>,
        game_version: Option<&'a str>,
    ) -> FomodContext<'a> {
        FomodContext {
            flags,
            game_version,
            data_dir: None,
            skse_version: None,
        }
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
                    conditional_type_patterns: Vec::new(),
                    condition_flags: HashMap::new(),
                },
                FomodOption {
                    name: "B".into(),
                    description: String::new(),
                    image: None,
                    files: Vec::new(),
                    type_descriptor: "Optional".into(),
                    conditional_type_patterns: Vec::new(),
                    condition_flags: HashMap::new(),
                },
            ],
        };

        let flags = HashMap::new();
        let ctx = test_ctx(&flags);
        let selected = default_selections_for_group(&group, &ctx);
        assert_eq!(selected, vec!["A".to_string(), "B".to_string()]);
    }

    /// FOMOD XML with condition flags and step visibility.
    const CONDITION_FLAGS_XML: &str = r#"
<config>
    <moduleName>Conditional Mod</moduleName>
    <installSteps order="Explicit">
        <installStep name="Choose Style">
            <optionalFileGroups order="Explicit">
                <group name="Style" type="SelectExactlyOne">
                    <plugins order="Explicit">
                        <plugin name="Dark">
                            <description>Dark theme</description>
                            <conditionFlags>
                                <flag name="style">dark</flag>
                            </conditionFlags>
                            <files>
                                <folder source="dark/base" destination="textures" priority="0" />
                            </files>
                            <typeDescriptor>
                                <type name="Recommended" />
                            </typeDescriptor>
                        </plugin>
                        <plugin name="Light">
                            <description>Light theme</description>
                            <conditionFlags>
                                <flag name="style">light</flag>
                            </conditionFlags>
                            <files>
                                <folder source="light/base" destination="textures" priority="0" />
                            </files>
                            <typeDescriptor>
                                <type name="Optional" />
                            </typeDescriptor>
                        </plugin>
                    </plugins>
                </group>
            </optionalFileGroups>
        </installStep>
        <installStep name="Dark Extras">
            <visible>
                <dependencies operator="And">
                    <flagDependency flag="style" value="dark"/>
                </dependencies>
            </visible>
            <optionalFileGroups order="Explicit">
                <group name="DarkPatches" type="SelectAll">
                    <plugins order="Explicit">
                        <plugin name="Dark Patch">
                            <description>Extra dark textures</description>
                            <files>
                                <folder source="dark/extras" destination="textures" priority="1" />
                            </files>
                            <typeDescriptor>
                                <type name="Required" />
                            </typeDescriptor>
                        </plugin>
                    </plugins>
                </group>
            </optionalFileGroups>
        </installStep>
        <installStep name="Light Extras">
            <visible>
                <dependencies operator="And">
                    <flagDependency flag="style" value="light"/>
                </dependencies>
            </visible>
            <optionalFileGroups order="Explicit">
                <group name="LightPatches" type="SelectAll">
                    <plugins order="Explicit">
                        <plugin name="Light Patch">
                            <description>Extra light textures</description>
                            <files>
                                <folder source="light/extras" destination="textures" priority="1" />
                            </files>
                            <typeDescriptor>
                                <type name="Required" />
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
    fn parse_condition_flags_on_plugins() {
        let installer = parse_fomod_xml(CONDITION_FLAGS_XML).unwrap();
        let step = &installer.steps[0];
        let style_group = &step.groups[0];

        let dark = &style_group.options[0];
        assert_eq!(dark.condition_flags.get("style"), Some(&"dark".to_string()));

        let light = &style_group.options[1];
        assert_eq!(
            light.condition_flags.get("style"),
            Some(&"light".to_string())
        );
    }

    #[test]
    fn parse_visible_on_steps() {
        let installer = parse_fomod_xml(CONDITION_FLAGS_XML).unwrap();

        // First step has no visibility condition.
        assert!(installer.steps[0].visible.is_none());

        // Second step requires style=dark.
        let dark_vis = installer.steps[1].visible.as_ref().unwrap();
        assert_eq!(dark_vis.operator, "And");
        assert_eq!(dark_vis.flags.len(), 1);
        assert_eq!(dark_vis.flags[0].flag, "style");
        assert_eq!(dark_vis.flags[0].value, "dark");

        // Third step requires style=light.
        let light_vis = installer.steps[2].visible.as_ref().unwrap();
        assert_eq!(light_vis.flags[0].flag, "style");
        assert_eq!(light_vis.flags[0].value, "light");
    }

    #[test]
    fn default_selections_skips_invisible_steps() {
        let installer = parse_fomod_xml(CONDITION_FLAGS_XML).unwrap();
        let selections = get_default_selections(&installer, None, None);

        assert_eq!(selections.get("Style"), Some(&vec!["Dark".to_string()]));
        assert!(selections.contains_key("DarkPatches"));
        assert!(!selections.contains_key("LightPatches"));
    }

    #[test]
    fn files_for_selections_respects_visibility() {
        let installer = parse_fomod_xml(CONDITION_FLAGS_XML).unwrap();
        let mut selections = HashMap::new();
        selections.insert("Style".to_string(), vec!["Dark".to_string()]);
        selections.insert("DarkPatches".to_string(), vec!["Dark Patch".to_string()]);
        selections.insert("LightPatches".to_string(), vec!["Light Patch".to_string()]);

        let files = get_files_for_selections(&installer, &selections, None, None);
        let sources: Vec<&str> = files.iter().map(|f| f.source.as_str()).collect();

        assert!(sources.contains(&"dark/base"));
        assert!(sources.contains(&"dark/extras"));
        assert!(!sources.contains(&"light/extras"));
    }

    #[test]
    fn condition_block_evaluate_and() {
        let block = ConditionBlock {
            operator: "And".to_string(),
            flags: vec![
                FlagDependency { flag: "a".into(), value: "1".into() },
                FlagDependency { flag: "b".into(), value: "2".into() },
            ],
            game_dependencies: vec![],
            file_dependencies: vec![],
            children: vec![],
        };
        let mut flags = HashMap::new();
        assert!(!block.evaluate(&test_ctx(&flags)));
        flags.insert("a".into(), "1".into());
        assert!(!block.evaluate(&test_ctx(&flags)));
        flags.insert("b".into(), "2".into());
        assert!(block.evaluate(&test_ctx(&flags)));
        flags.insert("b".into(), "3".into());
        assert!(!block.evaluate(&test_ctx(&flags)));
    }

    #[test]
    fn condition_block_evaluate_or() {
        let block = ConditionBlock {
            operator: "Or".to_string(),
            flags: vec![
                FlagDependency { flag: "a".into(), value: "1".into() },
                FlagDependency { flag: "b".into(), value: "2".into() },
            ],
            game_dependencies: vec![],
            file_dependencies: vec![],
            children: vec![],
        };
        let mut flags = HashMap::new();
        assert!(!block.evaluate(&test_ctx(&flags)));
        flags.insert("a".into(), "1".into());
        assert!(block.evaluate(&test_ctx(&flags)));
        flags.clear();
        flags.insert("b".into(), "2".into());
        assert!(block.evaluate(&test_ctx(&flags)));
    }

    #[test]
    fn condition_block_empty_flags_always_true() {
        let block = ConditionBlock {
            operator: "And".to_string(),
            flags: vec![],
            game_dependencies: vec![],
            file_dependencies: vec![],
            children: vec![],
        };
        let flags = HashMap::new();
        assert!(block.evaluate(&test_ctx(&flags)));
    }

    #[test]
    fn version_gte_comparisons() {
        assert!(version_gte("1.6.1170", "1.6.1170")); // equal
        assert!(version_gte("1.6.1170", "1.5.97")); // greater
        assert!(!version_gte("1.5.97", "1.6.1170")); // less
        assert!(version_gte("2.0.0", "1.99.99")); // major greater
        assert!(!version_gte("1.5.97", "1.6.0")); // minor less
        assert!(version_gte("1.6.1170", "1.6.640")); // patch greater
        // Unknown AE version with "x" wildcard — must pass all AE thresholds
        assert!(version_gte("1.6.x (Anniversary Edition, ~35.4 MB)", "1.5.97"));
        assert!(version_gte("1.6.x (Anniversary Edition, ~35.4 MB)", "1.6.0"));
        assert!(version_gte("1.6.x (Anniversary Edition, ~35.4 MB)", "1.6.640"));
        assert!(version_gte("1.6.x (Anniversary Edition, ~35.4 MB)", "1.6.1170"));
    }

    #[test]
    fn condition_block_game_dependency_and() {
        let block = ConditionBlock {
            operator: "And".to_string(),
            flags: vec![],
            game_dependencies: vec![GameDependency { version: "1.6.1130".into() }],
            file_dependencies: vec![],
            children: vec![],
        };
        let flags = HashMap::new();
        assert!(block.evaluate(&test_ctx_with_game(&flags, Some("1.6.1170"))));
        assert!(!block.evaluate(&test_ctx_with_game(&flags, Some("1.5.97"))));
        assert!(block.evaluate(&test_ctx(&flags)));
    }

    #[test]
    fn condition_block_game_dependency_or_with_flags() {
        let block = ConditionBlock {
            operator: "Or".to_string(),
            flags: vec![FlagDependency { flag: "useAE".into(), value: "true".into() }],
            game_dependencies: vec![GameDependency { version: "1.6.0".into() }],
            file_dependencies: vec![],
            children: vec![],
        };
        let mut flags = HashMap::new();
        flags.insert("useAE".into(), "true".into());
        assert!(block.evaluate(&test_ctx_with_game(&flags, Some("1.5.97"))));
        let empty = HashMap::new();
        assert!(block.evaluate(&test_ctx_with_game(&empty, Some("1.6.1170"))));
        assert!(!block.evaluate(&test_ctx_with_game(&empty, Some("1.5.97"))));
    }

    #[test]
    fn fomod_without_conditions_unchanged() {
        let installer = parse_fomod_xml(SAMPLE_XML).unwrap();
        for step in &installer.steps {
            assert!(step.visible.is_none());
        }
        for step in &installer.steps {
            for group in &step.groups {
                for option in &group.options {
                    assert!(option.condition_flags.is_empty());
                }
            }
        }
        let selections = get_default_selections(&installer, None, None);
        assert_eq!(selections.get("Textures"), Some(&vec!["High Res".to_string()]));
        assert_eq!(selections.get("Extras"), Some(&vec![]));
    }

    /// FOMOD XML with gameDependency conditions (typical SE/AE version selection).
    const GAME_DEP_XML: &str = r#"
<config>
    <moduleName>Version DLLs</moduleName>
    <installSteps order="Explicit">
        <installStep name="SE DLLs">
            <visible>
                <dependencies operator="And">
                    <gameDependency version="1.5.97"/>
                </dependencies>
            </visible>
            <optionalFileGroups order="Explicit">
                <group name="SE_Plugins" type="SelectAll">
                    <plugins order="Explicit">
                        <plugin name="SE Plugin">
                            <description>SE version</description>
                            <files>
                                <file source="SKSE/Plugins/plugin_se.dll" destination="SKSE/Plugins/plugin.dll" priority="0" />
                            </files>
                            <typeDescriptor><type name="Required" /></typeDescriptor>
                        </plugin>
                    </plugins>
                </group>
            </optionalFileGroups>
        </installStep>
        <installStep name="AE DLLs">
            <visible>
                <dependencies operator="And">
                    <gameDependency version="1.6.0"/>
                </dependencies>
            </visible>
            <optionalFileGroups order="Explicit">
                <group name="AE_Plugins" type="SelectAll">
                    <plugins order="Explicit">
                        <plugin name="AE Plugin">
                            <description>AE version</description>
                            <files>
                                <file source="SKSE/Plugins/plugin_ae.dll" destination="SKSE/Plugins/plugin.dll" priority="0" />
                            </files>
                            <typeDescriptor><type name="Required" /></typeDescriptor>
                        </plugin>
                    </plugins>
                </group>
            </optionalFileGroups>
        </installStep>
    </installSteps>
</config>
"#;

    #[test]
    fn parse_game_dependency_xml() {
        let installer = parse_fomod_xml(GAME_DEP_XML).unwrap();
        assert_eq!(installer.steps.len(), 2);

        // SE step has gameDependency version="1.5.97"
        let se_vis = installer.steps[0].visible.as_ref().unwrap();
        assert_eq!(se_vis.game_dependencies.len(), 1);
        assert_eq!(se_vis.game_dependencies[0].version, "1.5.97");
        assert!(se_vis.flags.is_empty());

        // AE step has gameDependency version="1.6.0"
        let ae_vis = installer.steps[1].visible.as_ref().unwrap();
        assert_eq!(ae_vis.game_dependencies.len(), 1);
        assert_eq!(ae_vis.game_dependencies[0].version, "1.6.0");
    }

    #[test]
    fn game_dep_selects_ae_for_ae_version() {
        let installer = parse_fomod_xml(GAME_DEP_XML).unwrap();
        let selections = get_default_selections(&installer, Some("1.6.1170"), None);
        assert!(selections.contains_key("AE_Plugins"));
        assert!(selections.contains_key("SE_Plugins"));
    }

    #[test]
    fn game_dep_only_se_for_se_version() {
        let installer = parse_fomod_xml(GAME_DEP_XML).unwrap();
        let selections = get_default_selections(&installer, Some("1.5.97"), None);
        assert!(selections.contains_key("SE_Plugins"));
        assert!(!selections.contains_key("AE_Plugins"));
    }

    #[test]
    fn game_dep_files_for_ae() {
        let installer = parse_fomod_xml(GAME_DEP_XML).unwrap();
        let selections = get_default_selections(&installer, Some("1.6.1170"), None);
        let files = get_files_for_selections(&installer, &selections, Some("1.6.1170"), None);
        let sources: Vec<&str> = files.iter().map(|f| f.source.as_str()).collect();
        assert!(sources.contains(&"SKSE/Plugins/plugin_ae.dll"));
    }

    #[test]
    fn game_dep_files_for_se() {
        let installer = parse_fomod_xml(GAME_DEP_XML).unwrap();
        let selections = get_default_selections(&installer, Some("1.5.97"), None);
        let files = get_files_for_selections(&installer, &selections, Some("1.5.97"), None);
        let sources: Vec<&str> = files.iter().map(|f| f.source.as_str()).collect();
        assert!(sources.contains(&"SKSE/Plugins/plugin_se.dll"));
        assert!(!sources.contains(&"SKSE/Plugins/plugin_ae.dll"));
    }

    #[test]
    fn fomod_cache_hit_returns_same_result() {
        let tmp = tempfile::tempdir().unwrap();
        let fomod_dir = tmp.path().join("fomod");
        std::fs::create_dir_all(&fomod_dir).unwrap();
        std::fs::write(fomod_dir.join("ModuleConfig.xml"), SAMPLE_XML).unwrap();

        let cache = new_fomod_cache();

        // First call: cache miss
        let result1 = parse_fomod_cached(&cache, "abc123", tmp.path()).unwrap();
        assert!(result1.is_some());
        assert_eq!(result1.as_ref().unwrap().module_name, "Test Mod");

        // Second call: cache hit (even if files were deleted)
        std::fs::remove_dir_all(&fomod_dir).unwrap();
        let result2 = parse_fomod_cached(&cache, "abc123", tmp.path()).unwrap();
        assert!(result2.is_some());
        assert_eq!(result2.as_ref().unwrap().module_name, "Test Mod");
    }

    #[test]
    fn fomod_cache_miss_for_different_key() {
        let tmp = tempfile::tempdir().unwrap();
        let fomod_dir = tmp.path().join("fomod");
        std::fs::create_dir_all(&fomod_dir).unwrap();
        std::fs::write(fomod_dir.join("ModuleConfig.xml"), SAMPLE_XML).unwrap();

        let cache = new_fomod_cache();

        // Populate cache for key "aaa"
        let _ = parse_fomod_cached(&cache, "aaa", tmp.path()).unwrap();

        // Different key "bbb" should be a cache miss and re-parse
        let result = parse_fomod_cached(&cache, "bbb", tmp.path()).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn fomod_cache_no_fomod_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = new_fomod_cache();

        // No fomod directory — returns None, does NOT cache
        let result = parse_fomod_cached(&cache, "empty", tmp.path()).unwrap();
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // fileDependency tests
    // -----------------------------------------------------------------------

    #[test]
    fn file_dependency_active_file_exists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("SkyUI_SE.esp"), "test").unwrap();

        let dep = FileDependency {
            file: "SkyUI_SE.esp".into(),
            state: "Active".into(),
        };
        assert!(evaluate_file_dependency(&dep, Some(tmp.path())));
    }

    #[test]
    fn file_dependency_active_file_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let dep = FileDependency {
            file: "SkyUI_SE.esp".into(),
            state: "Active".into(),
        };
        assert!(!evaluate_file_dependency(&dep, Some(tmp.path())));
    }

    #[test]
    fn file_dependency_missing_state() {
        let tmp = tempfile::tempdir().unwrap();
        // File doesn't exist → "Missing" state should be true.
        let dep = FileDependency {
            file: "NoMod.esp".into(),
            state: "Missing".into(),
        };
        assert!(evaluate_file_dependency(&dep, Some(tmp.path())));

        // File exists → "Missing" state should be false.
        std::fs::write(tmp.path().join("NoMod.esp"), "x").unwrap();
        assert!(!evaluate_file_dependency(&dep, Some(tmp.path())));
    }

    #[test]
    fn file_dependency_permissive_without_data_dir() {
        let dep = FileDependency {
            file: "anything.esp".into(),
            state: "Active".into(),
        };
        assert!(evaluate_file_dependency(&dep, None));
    }

    #[test]
    fn parse_file_dependency_xml() {
        let xml = r#"
<config>
    <moduleName>FileDep Test</moduleName>
    <installSteps order="Explicit">
        <installStep name="Check">
            <visible>
                <dependencies operator="And">
                    <fileDependency file="SkyUI_SE.esp" state="Active"/>
                </dependencies>
            </visible>
            <optionalFileGroups order="Explicit">
                <group name="Compat" type="SelectAll">
                    <plugins order="Explicit">
                        <plugin name="SkyUI Compat">
                            <description>Compat patch</description>
                            <files>
                                <file source="compat/skyui.esp" destination="skyui_compat.esp" priority="0"/>
                            </files>
                            <typeDescriptor><type name="Required"/></typeDescriptor>
                        </plugin>
                    </plugins>
                </group>
            </optionalFileGroups>
        </installStep>
    </installSteps>
</config>"#;
        let installer = parse_fomod_xml(xml).unwrap();
        let vis = installer.steps[0].visible.as_ref().unwrap();
        assert_eq!(vis.file_dependencies.len(), 1);
        assert_eq!(vis.file_dependencies[0].file, "SkyUI_SE.esp");
        assert_eq!(vis.file_dependencies[0].state, "Active");
    }

    #[test]
    fn file_dep_step_visibility_with_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("SkyUI_SE.esp"), "x").unwrap();

        let xml = r#"
<config>
    <moduleName>FD</moduleName>
    <installSteps order="Explicit">
        <installStep name="SkyUI Check">
            <visible>
                <dependencies operator="And">
                    <fileDependency file="SkyUI_SE.esp" state="Active"/>
                </dependencies>
            </visible>
            <optionalFileGroups order="Explicit">
                <group name="G" type="SelectAll">
                    <plugins order="Explicit">
                        <plugin name="P">
                            <description>d</description>
                            <files><file source="a" destination="b" priority="0"/></files>
                            <typeDescriptor><type name="Required"/></typeDescriptor>
                        </plugin>
                    </plugins>
                </group>
            </optionalFileGroups>
        </installStep>
    </installSteps>
</config>"#;
        let installer = parse_fomod_xml(xml).unwrap();

        // With data_dir that has the file → step visible.
        let sel = get_default_selections(&installer, None, Some(tmp.path()));
        assert!(sel.contains_key("G"));

        // Without the file → step hidden.
        let tmp2 = tempfile::tempdir().unwrap();
        let sel2 = get_default_selections(&installer, None, Some(tmp2.path()));
        assert!(!sel2.contains_key("G"));
    }

    // -----------------------------------------------------------------------
    // Nested composite condition tests
    // -----------------------------------------------------------------------

    #[test]
    fn nested_composite_or_with_and_children() {
        let xml = r#"
<config>
    <moduleName>Nested</moduleName>
    <installSteps order="Explicit">
        <installStep name="S1">
            <visible>
                <dependencies operator="Or">
                    <dependencies operator="And">
                        <flagDependency flag="A" value="1"/>
                        <flagDependency flag="B" value="2"/>
                    </dependencies>
                    <flagDependency flag="C" value="3"/>
                </dependencies>
            </visible>
            <optionalFileGroups order="Explicit">
                <group name="G" type="SelectAll">
                    <plugins order="Explicit">
                        <plugin name="P">
                            <description>d</description>
                            <files><file source="x" destination="y" priority="0"/></files>
                            <typeDescriptor><type name="Required"/></typeDescriptor>
                        </plugin>
                    </plugins>
                </group>
            </optionalFileGroups>
        </installStep>
    </installSteps>
</config>"#;
        let installer = parse_fomod_xml(xml).unwrap();
        let vis = installer.steps[0].visible.as_ref().unwrap();
        assert_eq!(vis.operator, "Or");
        assert_eq!(vis.children.len(), 1);
        assert_eq!(vis.flags.len(), 1); // C=3 at outer level
        assert_eq!(vis.children[0].operator, "And");
        assert_eq!(vis.children[0].flags.len(), 2);

        // Only C=3 → visible (Or: outer flag matches).
        let mut flags = HashMap::new();
        flags.insert("C".into(), "3".into());
        let sel = get_default_selections(&installer, None, None);
        // With no flags, nothing matches → step hidden.
        assert!(!sel.contains_key("G"));

        // Manually test with context.
        assert!(vis.evaluate(&test_ctx(&flags)));

        // A=1 + B=2 → visible (Or: child And matches).
        let mut flags2 = HashMap::new();
        flags2.insert("A".into(), "1".into());
        flags2.insert("B".into(), "2".into());
        assert!(vis.evaluate(&test_ctx(&flags2)));

        // Only A=1 → not visible (And child needs both, outer flag C not set).
        let mut flags3 = HashMap::new();
        flags3.insert("A".into(), "1".into());
        assert!(!vis.evaluate(&test_ctx(&flags3)));
    }

    // -----------------------------------------------------------------------
    // conditionalFileInstalls tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_conditional_file_installs_xml() {
        let xml = r#"
<config>
    <moduleName>CFI</moduleName>
    <installSteps order="Explicit">
        <installStep name="Choose">
            <optionalFileGroups order="Explicit">
                <group name="G" type="SelectExactlyOne">
                    <plugins order="Explicit">
                        <plugin name="SkyUI">
                            <description>d</description>
                            <conditionFlags><flag name="useSkyUI">On</flag></conditionFlags>
                            <files><file source="main.esp" destination="main.esp" priority="0"/></files>
                            <typeDescriptor><type name="Recommended"/></typeDescriptor>
                        </plugin>
                    </plugins>
                </group>
            </optionalFileGroups>
        </installStep>
    </installSteps>
    <conditionalFileInstalls>
        <patterns>
            <pattern>
                <dependencies operator="And">
                    <flagDependency flag="useSkyUI" value="On"/>
                </dependencies>
                <files>
                    <folder source="compat/skyui" destination="meshes" priority="0"/>
                </files>
            </pattern>
        </patterns>
    </conditionalFileInstalls>
</config>"#;
        let installer = parse_fomod_xml(xml).unwrap();
        assert_eq!(installer.conditional_file_installs.len(), 1);
        assert_eq!(installer.conditional_file_installs[0].files.len(), 1);
        assert_eq!(installer.conditional_file_installs[0].files[0].source, "compat/skyui");

        // When "SkyUI" selected → flag set → conditional files included.
        let mut sel = HashMap::new();
        sel.insert("G".into(), vec!["SkyUI".into()]);
        let files = get_files_for_selections(&installer, &sel, None, None);
        let sources: Vec<&str> = files.iter().map(|f| f.source.as_str()).collect();
        assert!(sources.contains(&"compat/skyui"));
        assert!(sources.contains(&"main.esp"));
    }

    // -----------------------------------------------------------------------
    // conditionalTypeDescriptor tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_dependency_type_xml() {
        let xml = r#"
<config>
    <moduleName>CDT</moduleName>
    <installSteps order="Explicit">
        <installStep name="S">
            <optionalFileGroups order="Explicit">
                <group name="G" type="SelectExactlyOne">
                    <plugins order="Explicit">
                        <plugin name="SkyUI Patch">
                            <description>d</description>
                            <files><file source="a" destination="b" priority="0"/></files>
                            <typeDescriptor>
                                <dependencyType>
                                    <defaultType name="Optional"/>
                                    <patterns>
                                        <pattern>
                                            <dependencies operator="And">
                                                <fileDependency file="SkyUI_SE.esp" state="Active"/>
                                            </dependencies>
                                            <type name="Recommended"/>
                                        </pattern>
                                    </patterns>
                                </dependencyType>
                            </typeDescriptor>
                        </plugin>
                        <plugin name="Other">
                            <description>d</description>
                            <files><file source="c" destination="d" priority="0"/></files>
                            <typeDescriptor><type name="Optional"/></typeDescriptor>
                        </plugin>
                    </plugins>
                </group>
            </optionalFileGroups>
        </installStep>
    </installSteps>
</config>"#;
        let installer = parse_fomod_xml(xml).unwrap();
        let opt = &installer.steps[0].groups[0].options[0];
        assert_eq!(opt.type_descriptor, "Optional"); // default
        assert_eq!(opt.conditional_type_patterns.len(), 1);
        assert_eq!(opt.conditional_type_patterns[0].option_type, "Recommended");

        // Without data_dir → permissive → "Recommended" matches.
        let flags = HashMap::new();
        let ctx = test_ctx(&flags);
        assert_eq!(opt.effective_type(&ctx), "Recommended");

        // With data_dir without file → "Optional" fallback.
        let tmp = tempfile::tempdir().unwrap();
        let ctx2 = FomodContext {
            flags: &flags,
            game_version: None,
            data_dir: Some(tmp.path()),
            skse_version: None,
        };
        assert_eq!(opt.effective_type(&ctx2), "Optional");

        // With data_dir with file → "Recommended".
        std::fs::write(tmp.path().join("SkyUI_SE.esp"), "x").unwrap();
        assert_eq!(opt.effective_type(&ctx2), "Recommended");

        // Default selection: with SkyUI → picks "SkyUI Patch" (Recommended over Optional).
        let sel = get_default_selections(&installer, None, Some(tmp.path()));
        assert_eq!(sel.get("G"), Some(&vec!["SkyUI Patch".to_string()]));

        // Without SkyUI → both Optional, picks first.
        let tmp2 = tempfile::tempdir().unwrap();
        let sel2 = get_default_selections(&installer, None, Some(tmp2.path()));
        assert_eq!(sel2.get("G"), Some(&vec!["SkyUI Patch".to_string()]));
    }

    // -----------------------------------------------------------------------
    // moduleDependencies tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_module_dependencies_xml() {
        let xml = r#"
<config>
    <moduleName>ModDep</moduleName>
    <moduleDependencies operator="And">
        <fileDependency file="Skyrim.esm" state="Active"/>
        <gameDependency version="1.6.0"/>
    </moduleDependencies>
    <installSteps order="Explicit">
        <installStep name="S">
            <optionalFileGroups order="Explicit">
                <group name="G" type="SelectAll">
                    <plugins order="Explicit">
                        <plugin name="P">
                            <description>d</description>
                            <files><file source="a" destination="b" priority="0"/></files>
                            <typeDescriptor><type name="Required"/></typeDescriptor>
                        </plugin>
                    </plugins>
                </group>
            </optionalFileGroups>
        </installStep>
    </installSteps>
</config>"#;
        let installer = parse_fomod_xml(xml).unwrap();
        assert!(installer.module_dependencies.is_some());
        let deps = installer.module_dependencies.as_ref().unwrap();
        assert_eq!(deps.file_dependencies.len(), 1);
        assert_eq!(deps.game_dependencies.len(), 1);

        // Check with correct game version + file.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Skyrim.esm"), "x").unwrap();
        let flags = HashMap::new();
        let ctx = FomodContext {
            flags: &flags,
            game_version: Some("1.6.1170"),
            data_dir: Some(tmp.path()),
            skse_version: None,
        };
        assert!(check_module_dependencies(&installer, &ctx).is_none());

        // Wrong game version.
        let ctx2 = FomodContext {
            flags: &flags,
            game_version: Some("1.5.97"),
            data_dir: Some(tmp.path()),
            skse_version: None,
        };
        assert!(check_module_dependencies(&installer, &ctx2).is_some());
    }

    // -----------------------------------------------------------------------
    // fommDependency and SE dependency tests (always-pass)
    // -----------------------------------------------------------------------

    #[test]
    fn fomm_and_se_deps_are_ignored_in_parsing() {
        let xml = r#"
<config>
    <moduleName>Compat</moduleName>
    <installSteps order="Explicit">
        <installStep name="S">
            <visible>
                <dependencies operator="And">
                    <fommDependency version="0.13.21"/>
                    <skseDependency version="2.0.0"/>
                    <flagDependency flag="x" value="1"/>
                </dependencies>
            </visible>
            <optionalFileGroups order="Explicit">
                <group name="G" type="SelectAll">
                    <plugins order="Explicit">
                        <plugin name="P">
                            <description>d</description>
                            <files><file source="a" destination="b" priority="0"/></files>
                            <typeDescriptor><type name="Required"/></typeDescriptor>
                        </plugin>
                    </plugins>
                </group>
            </optionalFileGroups>
        </installStep>
    </installSteps>
</config>"#;
        let installer = parse_fomod_xml(xml).unwrap();
        let vis = installer.steps[0].visible.as_ref().unwrap();
        // fommDependency and skseDependency are silently ignored.
        assert!(vis.file_dependencies.is_empty());
        assert!(vis.game_dependencies.is_empty());
        // Only the flagDependency was kept.
        assert_eq!(vis.flags.len(), 1);
        assert_eq!(vis.flags[0].flag, "x");

        // Without flag x=1, step is hidden (fomm/skse deps are always-pass,
        // but the flagDependency x=1 is not met).
        let sel = get_default_selections(&installer, None, None);
        assert!(!sel.contains_key("G"));
    }

    #[test]
    fn read_xml_utf16le() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let xml = "<config><moduleName>Test</moduleName></config>";
        // Write UTF-16 LE with BOM
        let mut bytes: Vec<u8> = vec![0xFF, 0xFE]; // BOM
        for c in xml.encode_utf16() {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        std::fs::write(tmp.path(), &bytes).unwrap();
        let result = read_xml_any_encoding(tmp.path()).unwrap();
        assert!(result.contains("<moduleName>Test</moduleName>"));
    }

    #[test]
    fn read_xml_utf8_bom() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let xml = "<config><moduleName>Test</moduleName></config>";
        let mut bytes: Vec<u8> = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
        bytes.extend_from_slice(xml.as_bytes());
        std::fs::write(tmp.path(), &bytes).unwrap();
        let result = read_xml_any_encoding(tmp.path()).unwrap();
        assert!(result.contains("<moduleName>Test</moduleName>"));
        assert!(!result.starts_with('\u{FEFF}')); // BOM stripped
    }

    #[test]
    fn parse_fomod_utf16le() {
        let tmp = tempfile::tempdir().unwrap();
        let fomod_dir = tmp.path().join("fomod");
        std::fs::create_dir_all(&fomod_dir).unwrap();
        let xml = SAMPLE_XML;
        // Write as UTF-16 LE
        let mut bytes: Vec<u8> = vec![0xFF, 0xFE];
        for c in xml.encode_utf16() {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        std::fs::write(fomod_dir.join("ModuleConfig.xml"), &bytes).unwrap();
        let result = parse_fomod(tmp.path()).unwrap();
        assert!(result.is_some());
        let installer = result.unwrap();
        assert_eq!(installer.module_name, "Test Mod");
    }

    // -----------------------------------------------------------------------
    // v0.8.8 — Backslash normalization, Data/ prefix stripping, selection
    // fallback, encoding declaration stripping, case-insensitive resolution
    // -----------------------------------------------------------------------

    /// FOMOD XML that uses Windows backslashes in source/destination paths.
    const BACKSLASH_XML: &str = r#"
<config>
    <moduleName>Backslash Test</moduleName>
    <requiredInstallFiles>
        <folder source="Core\meshes" destination="" priority="0" />
    </requiredInstallFiles>
    <installSteps order="Explicit">
        <installStep name="Options">
            <optionalFileGroups order="Explicit">
                <group name="Quality" type="SelectExactlyOne">
                    <plugins order="Explicit">
                        <plugin name="HQ">
                            <description>High quality</description>
                            <files>
                                <folder source="textures\high" destination="textures" priority="0" />
                            </files>
                            <typeDescriptor><type name="Recommended" /></typeDescriptor>
                        </plugin>
                        <plugin name="LQ">
                            <description>Low quality</description>
                            <files>
                                <folder source="textures\low" destination="textures" priority="0" />
                            </files>
                            <typeDescriptor><type name="Optional" /></typeDescriptor>
                        </plugin>
                    </plugins>
                </group>
            </optionalFileGroups>
        </installStep>
    </installSteps>
</config>
"#;

    #[test]
    fn backslash_paths_normalised_to_forward_slash() {
        let installer = parse_fomod_xml(BACKSLASH_XML).expect("parse should succeed");
        // Required files: source "Core\meshes" → "Core/meshes"
        assert_eq!(installer.required_files[0].source, "Core/meshes");
        // Option files: source "textures\high" → "textures/high"
        let hq = &installer.steps[0].groups[0].options[0];
        assert_eq!(hq.files[0].source, "textures/high");
    }

    /// FOMOD XML with "Data\" prefix in destinations.
    const DATA_PREFIX_XML: &str = r#"
<config>
    <moduleName>Data Prefix Test</moduleName>
    <installSteps order="Explicit">
        <installStep name="Step">
            <optionalFileGroups order="Explicit">
                <group name="Main" type="SelectAll">
                    <plugins order="Explicit">
                        <plugin name="Core">
                            <description>Core files</description>
                            <files>
                                <folder source="src\meshes" destination="Data\meshes" priority="0" />
                                <file source="plugin.esp" destination="Data\plugin.esp" priority="0" />
                                <folder source="root_stuff" destination="Data" priority="0" />
                            </files>
                            <typeDescriptor><type name="Required" /></typeDescriptor>
                        </plugin>
                    </plugins>
                </group>
            </optionalFileGroups>
        </installStep>
    </installSteps>
</config>
"#;

    #[test]
    fn data_prefix_stripped_from_destinations() {
        let installer = parse_fomod_xml(DATA_PREFIX_XML).expect("parse should succeed");
        let files = &installer.steps[0].groups[0].options[0].files;
        // "Data\meshes" → normalise backslash → "Data/meshes" → strip prefix → "meshes"
        assert_eq!(files[0].destination, "meshes");
        // "Data\plugin.esp" → "plugin.esp"
        assert_eq!(files[1].destination, "plugin.esp");
        // "Data" (bare) → ""
        assert_eq!(files[2].destination, "");
    }

    #[test]
    fn strip_data_prefix_edge_cases() {
        assert_eq!(strip_data_prefix(""), "");
        assert_eq!(strip_data_prefix("data"), "");
        assert_eq!(strip_data_prefix("Data"), "");
        assert_eq!(strip_data_prefix("DATA/"), "");
        assert_eq!(strip_data_prefix("Data/meshes"), "meshes");
        assert_eq!(strip_data_prefix("data/Textures/High"), "Textures/High");
        // "Database" should NOT be stripped
        assert_eq!(strip_data_prefix("Database/files"), "Database/files");
        // No prefix — pass through
        assert_eq!(strip_data_prefix("meshes/test"), "meshes/test");
    }

    #[test]
    fn case_insensitive_group_name_matching() {
        let installer = parse_fomod_xml(SAMPLE_XML).unwrap();
        // Provide selections with different casing: "textures" instead of "Textures"
        let mut selections = HashMap::new();
        selections.insert("textures".to_string(), vec!["Low Res".to_string()]);

        let files = get_files_for_selections(&installer, &selections, None, None);
        let sources: Vec<&str> = files.iter().map(|f| f.source.as_str()).collect();
        // Should still match the "Textures" group via case-insensitive lookup
        assert!(sources.contains(&"textures/low"));
    }

    #[test]
    fn unmatched_group_falls_back_to_defaults() {
        let installer = parse_fomod_xml(SAMPLE_XML).unwrap();
        // Empty selections — no groups matched at all
        let selections = HashMap::new();

        let files = get_files_for_selections(&installer, &selections, None, None);
        let sources: Vec<&str> = files.iter().map(|f| f.source.as_str()).collect();
        // Should fall back to default for SelectExactlyOne: "Recommended" = "High Res"
        assert!(sources.contains(&"core")); // required
        assert!(sources.contains(&"textures/high")); // default for SelectExactlyOne
        // SelectAny "Extras" with no Required/Recommended → nothing selected
        assert!(!sources.contains(&"optional/enb.ini"));
    }

    #[test]
    fn strip_xml_encoding_declaration_removes_encoding() {
        let xml = r#"<?xml version="1.0" encoding="Windows-1252"?><config></config>"#;
        let result = strip_xml_encoding_declaration(xml);
        assert!(!result.contains("encoding"));
        assert!(result.contains("<?xml"));
        assert!(result.contains("<config>"));
    }

    #[test]
    fn strip_xml_encoding_declaration_handles_single_quotes() {
        let xml = r#"<?xml version='1.0' encoding='utf-16'?><config></config>"#;
        let result = strip_xml_encoding_declaration(xml);
        assert!(!result.contains("encoding"));
        assert!(result.contains("<?xml"));
    }

    #[test]
    fn strip_xml_encoding_declaration_no_encoding_passthrough() {
        let xml = r#"<?xml version="1.0"?><config></config>"#;
        let result = strip_xml_encoding_declaration(xml);
        assert_eq!(result, xml);
    }

    #[test]
    fn strip_xml_encoding_declaration_no_header_passthrough() {
        let xml = r#"<config></config>"#;
        let result = strip_xml_encoding_declaration(xml);
        assert_eq!(result, xml);
    }

    #[test]
    fn resolve_path_case_insensitive_works() {
        let tmp = tempfile::tempdir().unwrap();
        // Create "Textures/High/diffuse.dds"
        let dir = tmp.path().join("Textures").join("High");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("diffuse.dds"), b"test").unwrap();

        // Resolve with different casing
        let resolved = resolve_path_case_insensitive(tmp.path(), "textures/high");
        assert!(resolved.is_some());
        assert!(resolved.unwrap().is_dir());

        // Resolve a file path
        let resolved = resolve_path_case_insensitive(tmp.path(), "textures/high/diffuse.dds");
        assert!(resolved.is_some());
        assert!(resolved.unwrap().is_file());

        // Non-existent path returns None
        let resolved = resolve_path_case_insensitive(tmp.path(), "nonexistent/path");
        assert!(resolved.is_none());
    }
}
