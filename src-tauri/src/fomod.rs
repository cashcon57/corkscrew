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

/// A composite condition block with an operator (`And` / `Or`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConditionBlock {
    /// `"And"` or `"Or"` — how child conditions are combined.
    pub operator: String,
    /// Individual flag checks.
    pub flags: Vec<FlagDependency>,
}

impl ConditionBlock {
    /// Evaluate this condition block against the current flag state.
    pub fn evaluate(&self, flags: &HashMap<String, String>) -> bool {
        if self.flags.is_empty() {
            return true;
        }
        match self.operator.as_str() {
            "Or" => self
                .flags
                .iter()
                .any(|dep| flags.get(&dep.flag) == Some(&dep.value)),
            // Default to And
            _ => self
                .flags
                .iter()
                .all(|dep| flags.get(&dep.flag) == Some(&dep.value)),
        }
    }
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
    /// Condition flags set when this option is selected.
    /// Maps flag name to flag value.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub condition_flags: HashMap<String, String>,
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
            // Use unescape_value to decode XML entities (&quot;, &amp;, etc.)
            return attr.unescape_value().ok().map(|s| s.into_owned());
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

/// Parse a `<visible>` or `<dependencies>` block into a [`ConditionBlock`].
///
/// Handles the structure:
/// ```xml
/// <visible>
///   <dependencies operator="And">
///     <flagDependency flag="name" value="val"/>
///   </dependencies>
/// </visible>
/// ```
/// Also works when called directly on `<dependencies>`.
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
                    let op = get_attr(e, "operator").unwrap_or_else(|| "And".to_string());
                    block = Some(ConditionBlock {
                        operator: op,
                        flags: Vec::new(),
                    });
                }
            }
            Ok(Event::Empty(ref e)) => {
                let local = e.local_name();
                if local.as_ref() == b"flagDependency" {
                    if let (Some(flag), Some(value)) = (get_attr(e, "flag"), get_attr(e, "value")) {
                        if let Some(ref mut b) = block {
                            b.flags.push(FlagDependency { flag, value });
                        }
                    }
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

    let xml_content = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read FOMOD config: {}", config_path.display()))?;

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
pub fn get_default_selections(installer: &FomodInstaller) -> HashMap<String, Vec<String>> {
    let mut selections = HashMap::new();
    let mut condition_flags: HashMap<String, String> = HashMap::new();

    for step in &installer.steps {
        // Skip steps whose visibility condition is not met.
        if !step_is_visible(step, &condition_flags) {
            continue;
        }

        for group in &step.groups {
            let selected = default_selections_for_group(group);
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
/// flags are tracked across steps to evaluate visibility. Files are returned
/// sorted by priority ascending (lowest first), so higher-priority files
/// overwrite lower-priority ones when extracted in order.
pub fn get_files_for_selections(
    installer: &FomodInstaller,
    selections: &HashMap<String, Vec<String>>,
) -> Vec<FomodFile> {
    let mut files: Vec<FomodFile> = installer.required_files.clone();
    let mut condition_flags: HashMap<String, String> = HashMap::new();

    for step in &installer.steps {
        // Skip steps whose visibility condition is not met.
        if !step_is_visible(step, &condition_flags) {
            continue;
        }

        for group in &step.groups {
            if let Some(selected_names) = selections.get(&group.name) {
                for option in &group.options {
                    if selected_names.contains(&option.name) {
                        files.extend(option.files.clone());
                        // Update condition flags from selected options.
                        for (k, v) in &option.condition_flags {
                            condition_flags.insert(k.clone(), v.clone());
                        }
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

/// Check whether a step's visibility condition is met.
/// Returns `true` if the step has no condition or if the condition evaluates to true.
fn step_is_visible(step: &FomodStep, flags: &HashMap<String, String>) -> bool {
    match &step.visible {
        None => true,
        Some(cond) => cond.evaluate(flags),
    }
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
                    condition_flags: HashMap::new(),
                },
                FomodOption {
                    name: "B".into(),
                    description: String::new(),
                    image: None,
                    files: Vec::new(),
                    type_descriptor: "Optional".into(),
                    condition_flags: HashMap::new(),
                },
            ],
        };

        let selected = default_selections_for_group(&group);
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
        // Default picks "Dark" (Recommended) → sets style=dark.
        // "Dark Extras" step becomes visible, "Light Extras" does not.
        let selections = get_default_selections(&installer);

        // Dark was selected.
        assert_eq!(selections.get("Style"), Some(&vec!["Dark".to_string()]));
        // DarkPatches step was visible → group was processed.
        assert!(selections.contains_key("DarkPatches"));
        // LightPatches step was NOT visible → group was NOT processed.
        assert!(!selections.contains_key("LightPatches"));
    }

    #[test]
    fn files_for_selections_respects_visibility() {
        let installer = parse_fomod_xml(CONDITION_FLAGS_XML).unwrap();
        let mut selections = HashMap::new();
        selections.insert("Style".to_string(), vec!["Dark".to_string()]);
        selections.insert("DarkPatches".to_string(), vec!["Dark Patch".to_string()]);
        // Even if someone passes LightPatches selections, step is invisible.
        selections.insert("LightPatches".to_string(), vec!["Light Patch".to_string()]);

        let files = get_files_for_selections(&installer, &selections);
        let sources: Vec<&str> = files.iter().map(|f| f.source.as_str()).collect();

        assert!(sources.contains(&"dark/base"));
        assert!(sources.contains(&"dark/extras"));
        // Light extras should NOT be included — step is invisible.
        assert!(!sources.contains(&"light/extras"));
    }

    #[test]
    fn condition_block_evaluate_and() {
        let block = ConditionBlock {
            operator: "And".to_string(),
            flags: vec![
                FlagDependency {
                    flag: "a".into(),
                    value: "1".into(),
                },
                FlagDependency {
                    flag: "b".into(),
                    value: "2".into(),
                },
            ],
        };
        let mut flags = HashMap::new();
        // Neither set.
        assert!(!block.evaluate(&flags));
        // Only one set.
        flags.insert("a".into(), "1".into());
        assert!(!block.evaluate(&flags));
        // Both set.
        flags.insert("b".into(), "2".into());
        assert!(block.evaluate(&flags));
        // Wrong value.
        flags.insert("b".into(), "3".into());
        assert!(!block.evaluate(&flags));
    }

    #[test]
    fn condition_block_evaluate_or() {
        let block = ConditionBlock {
            operator: "Or".to_string(),
            flags: vec![
                FlagDependency {
                    flag: "a".into(),
                    value: "1".into(),
                },
                FlagDependency {
                    flag: "b".into(),
                    value: "2".into(),
                },
            ],
        };
        let mut flags = HashMap::new();
        assert!(!block.evaluate(&flags));
        flags.insert("a".into(), "1".into());
        assert!(block.evaluate(&flags));
        flags.clear();
        flags.insert("b".into(), "2".into());
        assert!(block.evaluate(&flags));
    }

    #[test]
    fn condition_block_empty_flags_always_true() {
        let block = ConditionBlock {
            operator: "And".to_string(),
            flags: vec![],
        };
        assert!(block.evaluate(&HashMap::new()));
    }

    #[test]
    fn fomod_without_conditions_unchanged() {
        // Verify the original SAMPLE_XML still works identically.
        let installer = parse_fomod_xml(SAMPLE_XML).unwrap();

        // No visibility conditions on any step.
        for step in &installer.steps {
            assert!(step.visible.is_none());
        }

        // No condition flags on any option.
        for step in &installer.steps {
            for group in &step.groups {
                for option in &group.options {
                    assert!(option.condition_flags.is_empty());
                }
            }
        }

        // Default selections unchanged.
        let selections = get_default_selections(&installer);
        assert_eq!(
            selections.get("Textures"),
            Some(&vec!["High Res".to_string()])
        );
        assert_eq!(selections.get("Extras"), Some(&vec![]));
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
}
