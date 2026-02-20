use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read as IoRead;
use std::path::Path;

/// Master repository index: maps repo names to their modlists.json URLs
const REPOSITORIES_URL: &str =
    "https://raw.githubusercontent.com/wabbajack-tools/mod-lists/master/repositories.json";

// ---------------------------------------------------------------------------
// DTOs for the modlist gallery (fetched from Wabbajack's GitHub repos)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModlistLinks {
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub readme: String,
    #[serde(default)]
    pub download: String,
    #[serde(rename = "machineURL", default)]
    pub machine_url: String,
    #[serde(default, rename = "discordURL")]
    pub discord_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadMetadata {
    #[serde(rename = "Hash", default)]
    pub hash: String,
    #[serde(rename = "Size", default)]
    pub size: u64,
    #[serde(rename = "NumberOfArchives", default)]
    pub number_of_archives: u32,
    #[serde(rename = "SizeOfArchives", default)]
    pub size_of_archives: u64,
    #[serde(rename = "NumberOfInstalledFiles", default)]
    pub number_of_installed_files: u32,
    #[serde(rename = "SizeOfInstalledFiles", default)]
    pub size_of_installed_files: u64,
}

/// A single modlist entry as it appears in a repository's modlists.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModlistEntry {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub game: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub nsfw: bool,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub links: Option<ModlistLinks>,
    #[serde(default)]
    pub download_metadata: Option<DownloadMetadata>,
    /// Which repository this came from (populated after fetch).
    #[serde(default)]
    pub repository: String,
}

/// Flattened summary sent to the frontend for the gallery view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModlistSummary {
    pub title: String,
    pub description: String,
    pub author: String,
    pub game: String,
    pub tags: Vec<String>,
    pub nsfw: bool,
    pub version: String,
    pub image_url: String,
    pub readme_url: String,
    pub download_url: String,
    pub machine_url: String,
    pub repository: String,
    pub download_size: u64,
    pub install_size: u64,
    pub archive_count: u32,
    pub file_count: u32,
}

impl From<ModlistEntry> for ModlistSummary {
    fn from(e: ModlistEntry) -> Self {
        let links = e.links.unwrap_or(ModlistLinks {
            image: String::new(),
            readme: String::new(),
            download: String::new(),
            machine_url: String::new(),
            discord_url: String::new(),
        });
        let meta = e.download_metadata.unwrap_or(DownloadMetadata {
            hash: String::new(),
            size: 0,
            number_of_archives: 0,
            size_of_archives: 0,
            number_of_installed_files: 0,
            size_of_installed_files: 0,
        });
        ModlistSummary {
            title: e.title,
            description: e.description,
            author: e.author,
            game: e.game,
            tags: e.tags,
            nsfw: e.nsfw,
            version: e.version,
            image_url: links.image,
            readme_url: links.readme,
            download_url: links.download,
            machine_url: links.machine_url,
            repository: e.repository,
            download_size: meta.size_of_archives,
            install_size: meta.size_of_installed_files,
            archive_count: meta.number_of_archives,
            file_count: meta.number_of_installed_files,
        }
    }
}

// ---------------------------------------------------------------------------
// DTOs for a parsed .wabbajack file (the modlist JSON inside the ZIP)
// ---------------------------------------------------------------------------

/// Top-level modlist structure inside a .wabbajack file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct WabbajackModlist {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default, rename = "GameType")]
    pub game_type: u32,
    #[serde(default)]
    pub archives: Vec<WjArchive>,
    #[serde(default)]
    pub directives: Vec<serde_json::Value>,
    #[serde(default, rename = "IsNSFW")]
    pub is_nsfw: bool,
    #[serde(default)]
    pub wabbajack_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct WjArchive {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub state: serde_json::Value,
}

/// Directive type tag (extracted from `$type` field).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DirectiveKind {
    FromArchive,
    PatchedFromArchive,
    InlineFile,
    RemappedInlineFile,
    CreateBSA,
    TransformedTexture,
    MergedPatch,
    Unknown(String),
}

/// A simplified directive for display / analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectiveSummary {
    pub kind: String,
    pub to: String,
    pub size: u64,
}

/// Summary of a parsed .wabbajack file for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedModlist {
    pub name: String,
    pub author: String,
    pub description: String,
    pub version: String,
    pub game_type: u32,
    pub game_name: String,
    pub is_nsfw: bool,
    pub archive_count: usize,
    pub total_download_size: u64,
    pub directive_count: usize,
    pub directive_breakdown: HashMap<String, usize>,
    /// Archives with their download source type
    pub archives: Vec<ArchiveSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveSummary {
    pub name: String,
    pub size: u64,
    pub source_type: String,
    pub nexus_mod_id: Option<i64>,
    pub nexus_file_id: Option<i64>,
}

// ---------------------------------------------------------------------------
// Gallery fetching
// ---------------------------------------------------------------------------

/// Fetch the master repository index and all modlists from all repositories.
pub async fn fetch_modlist_gallery() -> Result<Vec<ModlistSummary>, String> {
    let client = reqwest::Client::builder()
        .user_agent("Corkscrew/0.3.0")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    // 1. Fetch repositories.json
    let repos: HashMap<String, String> = client
        .get(REPOSITORIES_URL)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch repositories index: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse repositories index: {}", e))?;

    // 2. Fetch each repository's modlists in parallel
    let mut handles = Vec::new();
    for (repo_name, repo_url) in &repos {
        let client = client.clone();
        let repo_name = repo_name.clone();
        let repo_url = repo_url.clone();
        handles.push(tokio::spawn(async move {
            fetch_single_repo(&client, &repo_name, &repo_url).await
        }));
    }

    let mut all_modlists = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Ok(entries)) => all_modlists.extend(entries),
            Ok(Err(e)) => log::warn!("Failed to fetch repository: {}", e),
            Err(e) => log::warn!("Task join error: {}", e),
        }
    }

    // Sort by title for consistent display
    all_modlists.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));

    Ok(all_modlists)
}

async fn fetch_single_repo(
    client: &reqwest::Client,
    repo_name: &str,
    repo_url: &str,
) -> Result<Vec<ModlistSummary>, String> {
    let response = client
        .get(repo_url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch repo '{}': {}", repo_name, e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Repo '{}' returned HTTP {}",
            repo_name,
            response.status()
        ));
    }

    let mut entries: Vec<ModlistEntry> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse repo '{}': {}", repo_name, e))?;

    // Tag each entry with its repository name
    for entry in &mut entries {
        entry.repository = repo_name.to_string();
    }

    Ok(entries.into_iter().map(ModlistSummary::from).collect())
}

// ---------------------------------------------------------------------------
// .wabbajack file parsing
// ---------------------------------------------------------------------------

/// Parse a local .wabbajack file and return a summary.
pub fn parse_wabbajack_file(path: &Path) -> Result<ParsedModlist, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Cannot open file: {}", e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Not a valid ZIP/.wabbajack file: {}", e))?;

    // Try "modlist" first, then "modlist.json"
    let modlist_json = read_zip_entry(&mut archive, "modlist")
        .or_else(|_| read_zip_entry(&mut archive, "modlist.json"))
        .map_err(|_| "No 'modlist' or 'modlist.json' entry found in .wabbajack file".to_string())?;

    let wj: WabbajackModlist = serde_json::from_str(&modlist_json)
        .map_err(|e| format!("Failed to parse modlist JSON: {}", e))?;

    // Build directive breakdown
    let mut directive_breakdown: HashMap<String, usize> = HashMap::new();
    for dir in &wj.directives {
        let kind = classify_directive(dir);
        *directive_breakdown.entry(kind).or_insert(0) += 1;
    }

    // Build archive summaries
    let archives: Vec<ArchiveSummary> = wj
        .archives
        .iter()
        .map(|a| {
            let (source_type, nexus_mod_id, nexus_file_id) = classify_archive_source(&a.state);
            ArchiveSummary {
                name: a.name.clone(),
                size: a.size,
                source_type,
                nexus_mod_id,
                nexus_file_id,
            }
        })
        .collect();

    let total_download_size: u64 = archives.iter().map(|a| a.size).sum();

    Ok(ParsedModlist {
        name: wj.name,
        author: wj.author,
        description: wj.description,
        version: wj.version,
        game_type: wj.game_type,
        game_name: game_type_name(wj.game_type),
        is_nsfw: wj.is_nsfw,
        archive_count: archives.len(),
        total_download_size,
        directive_count: wj.directives.len(),
        directive_breakdown,
        archives,
    })
}

fn read_zip_entry(
    archive: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
) -> Result<String, String> {
    let mut entry = archive
        .by_name(name)
        .map_err(|e| format!("Entry '{}' not found: {}", name, e))?;
    let mut buf = String::new();
    entry
        .read_to_string(&mut buf)
        .map_err(|e| format!("Failed to read entry '{}': {}", name, e))?;
    Ok(buf)
}

fn classify_directive(dir: &serde_json::Value) -> String {
    if let Some(type_str) = dir.get("$type").and_then(|v| v.as_str()) {
        let lower = type_str.to_lowercase();
        if lower.contains("patchedfromarchive") {
            "PatchedFromArchive".to_string()
        } else if lower.contains("fromarchive") {
            "FromArchive".to_string()
        } else if lower.contains("remappedinlinefile") {
            "RemappedInlineFile".to_string()
        } else if lower.contains("inlinefile") {
            "InlineFile".to_string()
        } else if lower.contains("createbsa") {
            "CreateBSA".to_string()
        } else if lower.contains("transformedtexture") {
            "TransformedTexture".to_string()
        } else if lower.contains("mergedpatch") {
            "MergedPatch".to_string()
        } else {
            type_str.to_string()
        }
    } else {
        "Unknown".to_string()
    }
}

fn classify_archive_source(state: &serde_json::Value) -> (String, Option<i64>, Option<i64>) {
    if let Some(type_str) = state.get("$type").and_then(|v| v.as_str()) {
        let lower = type_str.to_lowercase();
        if lower.contains("nexus") {
            let mod_id = state
                .get("ModID")
                .or(state.get("modID"))
                .and_then(|v| v.as_i64());
            let file_id = state
                .get("FileID")
                .or(state.get("fileID"))
                .and_then(|v| v.as_i64());
            ("Nexus".to_string(), mod_id, file_id)
        } else if lower.contains("http") {
            ("HTTP".to_string(), None, None)
        } else if lower.contains("googledrive") {
            ("GoogleDrive".to_string(), None, None)
        } else if lower.contains("mega") {
            ("Mega".to_string(), None, None)
        } else if lower.contains("mediafire") {
            ("MediaFire".to_string(), None, None)
        } else if lower.contains("moddb") {
            ("ModDB".to_string(), None, None)
        } else if lower.contains("wabbajackcdn") {
            ("WabbajackCDN".to_string(), None, None)
        } else if lower.contains("gamefile") {
            ("GameFile".to_string(), None, None)
        } else if lower.contains("manual") {
            ("Manual".to_string(), None, None)
        } else {
            (type_str.to_string(), None, None)
        }
    } else {
        ("Unknown".to_string(), None, None)
    }
}

/// Map Wabbajack's numeric GameType enum to a human-readable name.
fn game_type_name(game_type: u32) -> String {
    match game_type {
        0 => "Skyrim".to_string(),
        1 => "Skyrim Special Edition".to_string(),
        2 => "Fallout 4".to_string(),
        3 => "Oblivion".to_string(),
        4 => "Fallout New Vegas".to_string(),
        5 => "Skyrim VR".to_string(),
        6 => "Fallout 4 VR".to_string(),
        7 => "Morrowind".to_string(),
        8 => "Darkest Dungeon".to_string(),
        9 => "Fallout 3".to_string(),
        10 => "Enderal".to_string(),
        11 => "Enderal Special Edition".to_string(),
        12 => "Cyberpunk 2077".to_string(),
        13 => "Stardew Valley".to_string(),
        14 => "Kingdom Come Deliverance".to_string(),
        15 => "MechWarrior 5".to_string(),
        16 => "No Man's Sky".to_string(),
        17 => "Dragon Age Origins".to_string(),
        18 => "Dragon Age 2".to_string(),
        19 => "Dragon Age Inquisition".to_string(),
        20 => "Kerbal Space Program".to_string(),
        21 => "The Witcher 3".to_string(),
        22 => "Starfield".to_string(),
        23 => "Baldur's Gate 3".to_string(),
        24 => "Sims 4".to_string(),
        _ => format!("Unknown ({})", game_type),
    }
}

/// Map a Wabbajack game domain string (from gallery) to a display name.
pub fn game_domain_display(domain: &str) -> String {
    match domain {
        "skyrim" => "Skyrim LE".to_string(),
        "skyrimspecialedition" => "Skyrim SE".to_string(),
        "skyrimvr" | "skyrimvirtualreality" => "Skyrim VR".to_string(),
        "fallout4" => "Fallout 4".to_string(),
        "fallout4vr" => "Fallout 4 VR".to_string(),
        "falloutnewvegas" => "Fallout NV".to_string(),
        "fallout3" => "Fallout 3".to_string(),
        "oblivion" => "Oblivion".to_string(),
        "morrowind" => "Morrowind".to_string(),
        "enderal" => "Enderal".to_string(),
        "enderalspecialedition" => "Enderal SE".to_string(),
        "cyberpunk2077" => "Cyberpunk 2077".to_string(),
        "stardewvalley" => "Stardew Valley".to_string(),
        "witcher3" | "thewitcher3" => "Witcher 3".to_string(),
        "starfield" => "Starfield".to_string(),
        "baldursgate3" => "Baldur's Gate 3".to_string(),
        "dragonageinquisition" => "DA: Inquisition".to_string(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_game_type_name() {
        assert_eq!(game_type_name(1), "Skyrim Special Edition");
        assert_eq!(game_type_name(2), "Fallout 4");
        assert_eq!(game_type_name(99), "Unknown (99)");
    }

    #[test]
    fn test_game_domain_display() {
        assert_eq!(game_domain_display("skyrimspecialedition"), "Skyrim SE");
        assert_eq!(game_domain_display("fallout4"), "Fallout 4");
        assert_eq!(game_domain_display("unknown"), "unknown");
    }

    #[test]
    fn test_classify_directive() {
        let dir = serde_json::json!({
            "$type": "FromArchive, Wabbajack.DTOs"
        });
        assert_eq!(classify_directive(&dir), "FromArchive");

        let dir2 = serde_json::json!({
            "$type": "PatchedFromArchive, Wabbajack.DTOs"
        });
        assert_eq!(classify_directive(&dir2), "PatchedFromArchive");

        let dir3 = serde_json::json!({});
        assert_eq!(classify_directive(&dir3), "Unknown");
    }

    #[test]
    fn test_classify_archive_source() {
        let nexus = serde_json::json!({
            "$type": "Nexus, Wabbajack.DTOs",
            "ModID": 12604,
            "FileID": 35407,
        });
        let (kind, mod_id, file_id) = classify_archive_source(&nexus);
        assert_eq!(kind, "Nexus");
        assert_eq!(mod_id, Some(12604));
        assert_eq!(file_id, Some(35407));

        let http = serde_json::json!({
            "$type": "HttpDownloader, Wabbajack.DTOs",
        });
        let (kind, _, _) = classify_archive_source(&http);
        assert_eq!(kind, "HTTP");
    }

    #[test]
    fn test_modlist_entry_to_summary() {
        let entry = ModlistEntry {
            title: "Test List".to_string(),
            description: "A test".to_string(),
            author: "Author".to_string(),
            game: "skyrimspecialedition".to_string(),
            tags: vec!["gameplay".to_string()],
            nsfw: false,
            version: "1.0".to_string(),
            links: Some(ModlistLinks {
                image: "https://example.com/img.png".to_string(),
                readme: "https://example.com/readme".to_string(),
                download: "https://example.com/dl.wabbajack".to_string(),
                machine_url: "test_list".to_string(),
                discord_url: String::new(),
            }),
            download_metadata: Some(DownloadMetadata {
                hash: "abc=".to_string(),
                size: 1000,
                number_of_archives: 50,
                size_of_archives: 500000,
                number_of_installed_files: 10000,
                size_of_installed_files: 2000000,
            }),
            repository: "official".to_string(),
        };

        let summary: ModlistSummary = entry.into();
        assert_eq!(summary.title, "Test List");
        assert_eq!(summary.download_size, 500000);
        assert_eq!(summary.install_size, 2000000);
        assert_eq!(summary.archive_count, 50);
        assert_eq!(summary.file_count, 10000);
    }
}
