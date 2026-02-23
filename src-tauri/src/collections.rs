use std::collections::HashMap;
use std::path::Path;

use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const GRAPHQL_ENDPOINT: &str = "https://api.nexusmods.com/v2/graphql";

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum CollectionsError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("GraphQL error: {0}")]
    GraphQL(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Archive error: {0}")]
    Archive(String),

    #[error("Collection not found: {0}")]
    NotFound(String),
}

// ---------------------------------------------------------------------------
// Diff types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionDiff {
    pub collection_name: String,
    pub installed_revision: Option<u32>,
    pub latest_revision: u32,
    pub added: Vec<DiffEntry>,
    pub removed: Vec<DiffEntry>,
    pub updated: Vec<DiffUpdate>,
    pub unchanged: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiffEntry {
    pub name: String,
    pub version: String,
    pub source_type: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiffUpdate {
    pub name: String,
    pub installed_version: String,
    pub latest_version: String,
    pub source_type: String,
}

/// Compare an installed manifest against the latest revision mods from the API.
pub fn compute_diff(
    collection_name: &str,
    installed_revision: Option<u32>,
    latest_revision: u32,
    installed_mods: &[CollectionModEntry],
    latest_mods: &[CollectionMod],
) -> CollectionDiff {
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut updated = Vec::new();
    let mut unchanged = 0usize;

    // Index latest mods by nexus_mod_id (primary) and name (fallback)
    let mut latest_by_id: HashMap<i64, &CollectionMod> = HashMap::new();
    let mut latest_by_name: HashMap<String, &CollectionMod> = HashMap::new();
    for m in latest_mods {
        if let Some(id) = m.nexus_mod_id {
            latest_by_id.insert(id, m);
        }
        latest_by_name.insert(m.name.to_lowercase(), m);
    }

    // Track which latest mods have been matched
    let mut matched_latest: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Check each installed mod against latest
    for inst in installed_mods {
        let inst_mod_id = inst.source.mod_id;
        let matched = inst_mod_id
            .and_then(|id| latest_by_id.get(&id))
            .or_else(|| latest_by_name.get(&inst.name.to_lowercase()));

        if let Some(latest) = matched {
            matched_latest.insert(latest.name.to_lowercase());
            if inst.version != latest.version
                && !latest.version.is_empty()
                && !inst.version.is_empty()
            {
                updated.push(DiffUpdate {
                    name: latest.name.clone(),
                    installed_version: inst.version.clone(),
                    latest_version: latest.version.clone(),
                    source_type: latest.source_type.clone(),
                });
            } else {
                unchanged += 1;
            }
        } else {
            removed.push(DiffEntry {
                name: inst.name.clone(),
                version: inst.version.clone(),
                source_type: inst.source.source_type.clone(),
            });
        }
    }

    // Find mods in latest that aren't in installed
    for latest in latest_mods {
        if !matched_latest.contains(&latest.name.to_lowercase()) {
            added.push(DiffEntry {
                name: latest.name.clone(),
                version: latest.version.clone(),
                source_type: latest.source_type.clone(),
            });
        }
    }

    CollectionDiff {
        collection_name: collection_name.to_string(),
        installed_revision,
        latest_revision,
        added,
        removed,
        updated,
        unchanged,
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionInfo {
    pub slug: String,
    pub name: String,
    pub summary: String,
    pub description: String,
    pub author: String,
    pub game_domain: String,
    pub image_url: Option<String>,
    pub total_mods: usize,
    pub endorsements: u64,
    pub total_downloads: u64,
    pub latest_revision: u32,
    pub download_size: Option<u64>,
    pub updated_at: Option<String>,
    pub adult_content: bool,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionRevision {
    pub revision_number: u32,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub changelog: Option<String>,
    pub mod_count: usize,
    pub download_size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionMod {
    pub name: String,
    pub version: String,
    pub optional: bool,
    pub source_type: String,
    pub nexus_mod_id: Option<i64>,
    pub nexus_file_id: Option<i64>,
    pub download_url: Option<String>,
    pub instructions: Option<String>,
    pub file_size: Option<u64>,
    pub author: Option<String>,
    pub image_url: Option<String>,
    pub adult_content: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionManifest {
    pub name: String,
    pub author: String,
    pub description: String,
    pub game_domain: String,
    pub mods: Vec<CollectionModEntry>,
    #[serde(default, rename = "modRules")]
    pub mod_rules: Vec<CollectionModRule>,
    #[serde(default)]
    pub plugins: Vec<CollectionPlugin>,
    #[serde(default, rename = "installInstructions")]
    pub install_instructions: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub revision: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionModEntry {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub optional: bool,
    pub source: CollectionSource,
    #[serde(default)]
    pub choices: Option<serde_json::Value>,
    #[serde(default)]
    pub patches: Option<HashMap<String, String>>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub phase: Option<u32>,
    #[serde(default, rename = "fileOverrides")]
    pub file_overrides: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionSource {
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default, rename = "modId")]
    pub mod_id: Option<i64>,
    #[serde(default, rename = "fileId")]
    pub file_id: Option<i64>,
    #[serde(default, rename = "updatePolicy")]
    pub update_policy: Option<String>,
    #[serde(default)]
    pub md5: Option<String>,
    #[serde(default, rename = "fileSize")]
    pub file_size: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionModRule {
    pub source: ModReference,
    #[serde(rename = "type")]
    pub rule_type: String,
    pub reference: ModReference,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModReference {
    #[serde(default, rename = "fileMD5")]
    pub file_md5: Option<String>,
    #[serde(default, rename = "logicalFileName")]
    pub logical_file_name: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default, rename = "idHint")]
    pub id_hint: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionPlugin {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionSearchResult {
    pub collections: Vec<CollectionInfo>,
    pub total_count: u64,
}

// ---------------------------------------------------------------------------
// Internal GraphQL helper
// ---------------------------------------------------------------------------

async fn graphql_query<T: serde::de::DeserializeOwned>(
    api_key: Option<&str>,
    query: &str,
    variables: serde_json::Value,
) -> Result<T, CollectionsError> {
    // Build default headers with NexusMods API compliance fields
    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("application/json"));
    headers.insert("Application-Name", HeaderValue::from_static("Corkscrew"));
    let app_version = HeaderValue::from_str(env!("CARGO_PKG_VERSION"))
        .unwrap_or_else(|_| HeaderValue::from_static("0.0.0"));
    headers.insert("Application-Version", app_version);
    headers.insert("Protocol-Version", HeaderValue::from_static("0.15.5"));

    let client = reqwest::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let body = serde_json::json!({
        "query": query,
        "variables": variables,
    });

    let mut request = client.post(GRAPHQL_ENDPOINT);

    if let Some(key) = api_key {
        request = request.header("apikey", key);
    }

    let response = request.json(&body).send().await?;
    let status = response.status();

    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(CollectionsError::GraphQL(format!(
            "HTTP {}: {}",
            status, text
        )));
    }

    let json: serde_json::Value = response.json().await?;

    // Check for GraphQL-level errors
    if let Some(errors) = json.get("errors") {
        if let Some(arr) = errors.as_array() {
            if !arr.is_empty() {
                let messages: Vec<String> = arr
                    .iter()
                    .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                    .map(String::from)
                    .collect();
                return Err(CollectionsError::GraphQL(messages.join("; ")));
            }
        }
    }

    let data = json
        .get("data")
        .ok_or_else(|| CollectionsError::GraphQL("No 'data' field in response".into()))?;

    let result: T = serde_json::from_value(data.clone())?;
    Ok(result)
}

// ---------------------------------------------------------------------------
// GraphQL query strings
// ---------------------------------------------------------------------------

const BROWSE_COLLECTIONS_QUERY: &str = r#"
query CollectionsV2($count: Int, $offset: Int, $filter: CollectionsSearchFilter, $sort: [CollectionsSearchSort!]) {
    collectionsV2(count: $count, offset: $offset, filter: $filter, sort: $sort) {
        nodes {
            slug name summary
            user { name }
            tileImage { thumbnailUrl(size: small) }
            endorsements totalDownloads
            latestPublishedRevision { revisionNumber totalSize modCount }
            updatedAt adultContent
            tags { name }
            category { name }
            game { domainName }
        }
        nodesCount totalCount
    }
}
"#;

const GET_COLLECTION_QUERY: &str = r#"
query Collection($slug: String!, $viewAdultContent: Boolean) {
    collection(slug: $slug, viewAdultContent: $viewAdultContent) {
        slug name summary description
        user { name }
        tileImage { thumbnailUrl(size: small) }
        endorsements totalDownloads
        latestPublishedRevision { revisionNumber totalSize modCount }
        updatedAt adultContent
        tags { name }
        category { name }
        game { domainName }
    }
}
"#;

const GET_REVISIONS_QUERY: &str = r#"
query CollectionRevisions($slug: String!, $viewAdultContent: Boolean) {
    collection(slug: $slug, viewAdultContent: $viewAdultContent) {
        revisions {
            revisionNumber createdAt updatedAt
            revisionStatus totalSize
            modCount
        }
    }
}
"#;

const GET_REVISION_MODS_QUERY: &str = r#"
query RevisionMods($slug: String!, $revision: Int, $viewAdultContent: Boolean) {
    collectionRevision(slug: $slug, revision: $revision, viewAdultContent: $viewAdultContent) {
        modFiles {
            fileId optional
            file {
                fileId name size sizeInBytes version
                mod {
                    modId name author summary version pictureUrl adult
                    game { domainName }
                }
            }
        }
        externalResources { id name resourceType resourceUrl }
    }
}
"#;

// ---------------------------------------------------------------------------
// Public API functions
// ---------------------------------------------------------------------------

/// Search/browse collections for a game domain.
pub async fn browse_collections(
    api_key: Option<&str>,
    game_domain: &str,
    count: u32,
    offset: u32,
    sort_field: &str,
    sort_direction: &str,
    search_text: Option<&str>,
    author: Option<&str>,
    min_downloads: Option<i64>,
    min_endorsements: Option<i64>,
) -> Result<CollectionSearchResult, CollectionsError> {
    // Map friendly sort names to GraphQL field names
    // Note: NexusMods removed "totalDownloads" from CollectionsSearchSort;
    // "downloads" now falls back to endorsements as the best popularity proxy
    let gql_sort_key = match sort_field {
        "name" => "name",
        "downloads" => "endorsements",
        "rating" => "recentRating",
        "created" => "createdAt",
        "updated" => "updatedAt",
        _ => "endorsements",
    };
    let gql_direction = if sort_direction == "asc" {
        "ASC"
    } else {
        "DESC"
    };

    // Build sort array: NexusMods expects [{ "fieldName": { "direction": "DESC" } }]
    let mut direction_obj = serde_json::Map::new();
    direction_obj.insert(
        "direction".to_string(),
        serde_json::Value::String(gql_direction.to_string()),
    );
    let mut sort_obj = serde_json::Map::new();
    sort_obj.insert(
        gql_sort_key.to_string(),
        serde_json::Value::Object(direction_obj),
    );
    let sort_array = serde_json::Value::Array(vec![serde_json::Value::Object(sort_obj)]);

    // Build filter with required gameDomain + optional name search
    let mut filter = serde_json::Map::new();
    filter.insert(
        "gameDomain".to_string(),
        serde_json::json!([{ "op": "EQUALS", "value": game_domain }]),
    );
    if let Some(text) = search_text {
        filter.insert(
            "name".to_string(),
            serde_json::json!([{ "op": "WILDCARD", "value": format!("*{}*", text) }]),
        );
    }
    if let Some(auth) = author {
        if !auth.is_empty() {
            filter.insert(
                "authorName".to_string(),
                serde_json::json!([{ "op": "WILDCARD", "value": format!("*{}*", auth) }]),
            );
        }
    }
    if let Some(min_dl) = min_downloads {
        filter.insert(
            "totalDownloads".to_string(),
            serde_json::json!([{ "op": "GREATER_THAN", "value": min_dl }]),
        );
    }
    if let Some(min_end) = min_endorsements {
        filter.insert(
            "endorsements".to_string(),
            serde_json::json!([{ "op": "GREATER_THAN", "value": min_end }]),
        );
    }

    let variables = serde_json::json!({
        "count": count,
        "offset": offset,
        "filter": filter,
        "sort": sort_array,
    });

    let data: serde_json::Value =
        graphql_query(api_key, BROWSE_COLLECTIONS_QUERY, variables).await?;

    let collections_node = data
        .get("collectionsV2")
        .ok_or_else(|| CollectionsError::GraphQL("Missing 'collectionsV2' in response".into()))?;

    let total_count = collections_node
        .get("totalCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let nodes = collections_node
        .get("nodes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let collections: Vec<CollectionInfo> = nodes.iter().map(parse_collection_node).collect();

    Ok(CollectionSearchResult {
        collections,
        total_count,
    })
}

/// Get detailed info about a specific collection.
pub async fn get_collection(
    api_key: Option<&str>,
    slug: &str,
    _game_domain: &str,
) -> Result<CollectionInfo, CollectionsError> {
    let variables = serde_json::json!({
        "slug": slug,
        "viewAdultContent": true,
    });

    let data: serde_json::Value = graphql_query(api_key, GET_COLLECTION_QUERY, variables).await?;

    let node = data
        .get("collection")
        .ok_or_else(|| CollectionsError::NotFound(slug.to_string()))?;

    if node.is_null() {
        return Err(CollectionsError::NotFound(slug.to_string()));
    }

    Ok(parse_collection_node(node))
}

/// Get revisions for a collection.
pub async fn get_revisions(
    api_key: Option<&str>,
    slug: &str,
) -> Result<Vec<CollectionRevision>, CollectionsError> {
    let variables = serde_json::json!({
        "slug": slug,
        "viewAdultContent": true,
    });

    let data: serde_json::Value = graphql_query(api_key, GET_REVISIONS_QUERY, variables).await?;

    let collection = data
        .get("collection")
        .ok_or_else(|| CollectionsError::NotFound(slug.to_string()))?;

    if collection.is_null() {
        return Err(CollectionsError::NotFound(slug.to_string()));
    }

    let revisions = collection
        .get("revisions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let result: Vec<CollectionRevision> = revisions
        .iter()
        .map(|rev| {
            let mod_files = rev.get("modCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

            CollectionRevision {
                revision_number: rev
                    .get("revisionNumber")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                created_at: rev
                    .get("createdAt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                updated_at: rev
                    .get("updatedAt")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                changelog: None,
                mod_count: mod_files,
                download_size: rev.get("totalSize").and_then(|v| v.as_u64()).unwrap_or(0),
            }
        })
        .collect();

    Ok(result)
}

/// Get the mod list for a specific revision.
pub async fn get_revision_mods(
    api_key: Option<&str>,
    slug: &str,
    revision: u32,
) -> Result<Vec<CollectionMod>, CollectionsError> {
    let variables = serde_json::json!({
        "slug": slug,
        "revision": revision,
        "viewAdultContent": true,
    });

    let data: serde_json::Value =
        graphql_query(api_key, GET_REVISION_MODS_QUERY, variables).await?;

    let revision_node = data
        .get("collectionRevision")
        .ok_or_else(|| CollectionsError::NotFound(format!("{}@{}", slug, revision)))?;

    if revision_node.is_null() {
        return Err(CollectionsError::NotFound(format!("{}@{}", slug, revision)));
    }

    let mut mods = Vec::new();

    // Parse Nexus mod files
    if let Some(mod_files) = revision_node.get("modFiles").and_then(|v| v.as_array()) {
        for mf in mod_files {
            let optional = mf
                .get("optional")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let outer_file_id = mf.get("fileId").and_then(|v| v.as_i64());

            if let Some(file) = mf.get("file") {
                // Use outer modFiles.fileId, falling back to nested file.fileId
                let file_id = outer_file_id.or_else(|| file.get("fileId").and_then(|v| v.as_i64()));
                let file_size = file
                    .get("sizeInBytes")
                    .and_then(|v| v.as_u64())
                    .or_else(|| file.get("size").and_then(|v| v.as_u64()));
                let file_version = file
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if let Some(mod_info) = file.get("mod") {
                    let name = mod_info
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown")
                        .to_string();
                    let author = mod_info
                        .get("author")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let picture_url = mod_info
                        .get("pictureUrl")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let adult = mod_info
                        .get("adult")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let mod_id = mod_info.get("modId").and_then(|v| v.as_i64());

                    mods.push(CollectionMod {
                        name,
                        version: file_version,
                        optional,
                        source_type: "nexus".to_string(),
                        nexus_mod_id: mod_id,
                        nexus_file_id: file_id,
                        download_url: None,
                        instructions: None,
                        file_size,
                        author,
                        image_url: picture_url,
                        adult_content: adult,
                    });
                }
            }
        }
    }

    // Parse external resources
    if let Some(externals) = revision_node
        .get("externalResources")
        .and_then(|v| v.as_array())
    {
        for ext in externals {
            let name = ext
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("External Resource")
                .to_string();
            let resource_type = ext
                .get("resourceType")
                .and_then(|v| v.as_str())
                .unwrap_or("browse")
                .to_string();
            let resource_url = ext
                .get("resourceUrl")
                .and_then(|v| v.as_str())
                .map(String::from);

            mods.push(CollectionMod {
                name,
                version: String::new(),
                optional: true,
                source_type: classify_external_source(&resource_type),
                nexus_mod_id: None,
                nexus_file_id: None,
                download_url: resource_url,
                instructions: None,
                file_size: None,
                author: None,
                image_url: None,
                adult_content: false,
            });
        }
    }

    Ok(mods)
}

/// Parse a collection.json manifest from a downloaded collection bundle (7z).
pub fn parse_collection_bundle(bundle_path: &Path) -> Result<CollectionManifest, CollectionsError> {
    use std::path::PathBuf;

    let file = std::fs::File::open(bundle_path).map_err(CollectionsError::Io)?;

    // sevenz-rust requires a destination path even when using a custom extract
    // callback. We use a temp directory and skip writing files we don't need.
    let temp_dir = std::env::temp_dir().join("corkscrew_collection_extract");
    let mut json_data: Option<Vec<u8>> = None;

    sevenz_rust::decompress_with_extract_fn(
        file,
        &temp_dir,
        |entry: &sevenz_rust::SevenZArchiveEntry,
         reader: &mut dyn std::io::Read,
         _dest: &PathBuf|
         -> Result<bool, sevenz_rust::Error> {
            let entry_name = entry.name();
            if entry_name == "collection.json" || entry_name.ends_with("/collection.json") {
                let mut data = Vec::new();
                reader.read_to_end(&mut data)?;
                json_data = Some(data);
                // Return false to skip the default file-write behaviour
                return Ok(false);
            }
            // Skip all other entries (don't extract to disk)
            Ok(false)
        },
    )
    .map_err(|e| CollectionsError::Archive(format!("Failed to read 7z archive: {}", e)))?;

    // Clean up temp dir (best-effort)
    let _ = std::fs::remove_dir_all(&temp_dir);

    let data = json_data
        .ok_or_else(|| CollectionsError::Archive("No collection.json found in archive".into()))?;

    let json_str = String::from_utf8(data).map_err(|e| {
        CollectionsError::Archive(format!("Invalid UTF-8 in collection.json: {}", e))
    })?;

    parse_collection_json(&json_str)
}

/// Parse a raw collection.json string.
pub fn parse_collection_json(json_str: &str) -> Result<CollectionManifest, CollectionsError> {
    let manifest: CollectionManifest = serde_json::from_str(json_str)?;
    Ok(manifest)
}

/// Apply mod rules to determine installation order.
///
/// Uses a topological sort over the mod rules to figure out which mods need
/// to be installed before others. Returns a vector of indices into
/// `manifest.mods` representing the resolved installation order.
pub fn resolve_install_order(manifest: &CollectionManifest) -> Vec<usize> {
    let mod_count = manifest.mods.len();
    if mod_count == 0 {
        return Vec::new();
    }

    // Build a name-to-index lookup so we can map rule references to mod entries.
    let name_to_idx: HashMap<String, usize> = manifest
        .mods
        .iter()
        .enumerate()
        .map(|(i, m)| (m.name.to_lowercase(), i))
        .collect();

    // Also build an md5-to-index and logical-name-to-index lookup.
    let mut md5_to_idx: HashMap<String, usize> = HashMap::new();
    let mut logical_to_idx: HashMap<String, usize> = HashMap::new();

    for (i, m) in manifest.mods.iter().enumerate() {
        if let Some(ref md5) = m.source.md5 {
            md5_to_idx.insert(md5.to_lowercase(), i);
        }
        // Use the mod name as a fallback logical name
        logical_to_idx.insert(m.name.to_lowercase(), i);
    }

    // Build a directed graph: edges[a] contains b means "a must be installed before b".
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); mod_count];
    let mut in_degree: Vec<usize> = vec![0; mod_count];

    let resolve_ref = |r: &ModReference| -> Option<usize> {
        if let Some(ref md5) = r.file_md5 {
            if let Some(&idx) = md5_to_idx.get(&md5.to_lowercase()) {
                return Some(idx);
            }
        }
        if let Some(ref name) = r.logical_file_name {
            if let Some(&idx) = logical_to_idx.get(&name.to_lowercase()) {
                return Some(idx);
            }
        }
        if let Some(ref hint) = r.id_hint {
            if let Some(&idx) = name_to_idx.get(&hint.to_lowercase()) {
                return Some(idx);
            }
        }
        if let Some(ref tag) = r.tag {
            if let Some(&idx) = name_to_idx.get(&tag.to_lowercase()) {
                return Some(idx);
            }
        }
        None
    };

    for rule in &manifest.mod_rules {
        let src = resolve_ref(&rule.source);
        let dst = resolve_ref(&rule.reference);

        if let (Some(s), Some(d)) = (src, dst) {
            if s == d {
                continue;
            }
            match rule.rule_type.as_str() {
                "before" => {
                    // source should be installed before reference
                    edges[s].push(d);
                    in_degree[d] += 1;
                }
                "after" => {
                    // source should be installed after reference
                    edges[d].push(s);
                    in_degree[s] += 1;
                }
                "requires" => {
                    // reference must be installed before source
                    edges[d].push(s);
                    in_degree[s] += 1;
                }
                _ => {
                    // "conflicts", "recommends", etc. - no ordering constraint
                }
            }
        }
    }

    // Group mods by phase first, then topologically sort within each phase.
    let mut phase_map: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, m) in manifest.mods.iter().enumerate() {
        let phase = m.phase.unwrap_or(0);
        phase_map.entry(phase).or_default().push(i);
    }

    let mut phases: Vec<u32> = phase_map.keys().copied().collect();
    phases.sort();

    let mut result = Vec::with_capacity(mod_count);

    for phase in phases {
        let members = &phase_map[&phase];
        let member_set: std::collections::HashSet<usize> = members.iter().copied().collect();

        // Kahn's algorithm scoped to this phase
        let mut local_in: Vec<usize> = in_degree.clone();
        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();

        for &idx in members {
            // Only count edges from within the same phase for in-degree
            let external_in: usize = edges
                .iter()
                .enumerate()
                .filter(|(src, targets)| !member_set.contains(src) && targets.contains(&idx))
                .count();
            let total_in = local_in[idx];
            let phase_in = total_in.saturating_sub(external_in);
            local_in[idx] = phase_in;

            if phase_in == 0 {
                queue.push_back(idx);
            }
        }

        let mut phase_order = Vec::new();
        while let Some(node) = queue.pop_front() {
            phase_order.push(node);
            for &neighbor in &edges[node] {
                if member_set.contains(&neighbor) {
                    local_in[neighbor] = local_in[neighbor].saturating_sub(1);
                    if local_in[neighbor] == 0 {
                        queue.push_back(neighbor);
                    }
                }
            }
        }

        // Append any mods not reached by topological sort (cycle fallback)
        for &idx in members {
            if !phase_order.contains(&idx) {
                phase_order.push(idx);
            }
        }

        result.extend(phase_order);
    }

    result
}

/// Get game domain display name.
pub fn game_domain_display(domain: &str) -> &str {
    match domain {
        "skyrim" => "Skyrim LE",
        "skyrimspecialedition" => "Skyrim SE",
        "skyrimvr" => "Skyrim VR",
        "fallout4" => "Fallout 4",
        "fallout4vr" => "Fallout 4 VR",
        "falloutnewvegas" => "Fallout NV",
        "fallout3" => "Fallout 3",
        "oblivion" => "Oblivion",
        "morrowind" => "Morrowind",
        "enderal" => "Enderal",
        "enderalspecialedition" => "Enderal SE",
        "cyberpunk2077" => "Cyberpunk 2077",
        "stardewvalley" => "Stardew Valley",
        "witcher3" => "Witcher 3",
        "starfield" => "Starfield",
        "baldursgate3" => "Baldur's Gate 3",
        "dragonageinquisition" => "DA: Inquisition",
        "dragonage2" => "Dragon Age 2",
        "dragonageorigins" => "Dragon Age: Origins",
        "nomanssky" => "No Man's Sky",
        "mountandblade2bannerlord" => "Bannerlord",
        "valheim" => "Valheim",
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Parse a single collection node from the GraphQL response into a
/// `CollectionInfo`.
fn parse_collection_node(node: &serde_json::Value) -> CollectionInfo {
    let slug = node
        .get("slug")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let name = node
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let summary = node
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let description = node
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let author = node
        .get("user")
        .and_then(|u| u.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();

    let game_domain = node
        .get("game")
        .and_then(|g| g.get("domainName"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let image_url = node
        .get("tileImage")
        .and_then(|t| t.get("thumbnailUrl"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let endorsements = node
        .get("endorsements")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let total_downloads = node
        .get("totalDownloads")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let latest_pub_rev = node.get("latestPublishedRevision");

    let latest_revision = latest_pub_rev
        .and_then(|r| r.get("revisionNumber"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let download_size = latest_pub_rev
        .and_then(|r| r.get("totalSize"))
        .and_then(|v| v.as_u64())
        .filter(|&s| s > 0);

    let updated_at = node
        .get("updatedAt")
        .and_then(|v| v.as_str())
        .map(String::from);

    let adult_content = node
        .get("adultContent")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let tags: Vec<String> = node
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Count total mods from latest published revision's modCount scalar
    let total_mods = latest_pub_rev
        .and_then(|r| r.get("modCount"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    CollectionInfo {
        slug,
        name,
        summary,
        description,
        author,
        game_domain,
        image_url,
        total_mods,
        endorsements,
        total_downloads,
        latest_revision,
        download_size,
        updated_at,
        adult_content,
        tags,
    }
}

/// Classify an external resource type string into our canonical source types.
fn classify_external_source(resource_type: &str) -> String {
    match resource_type.to_lowercase().as_str() {
        "direct" | "directdownload" => "direct".to_string(),
        "browse" | "website" => "browse".to_string(),
        "manual" => "manual".to_string(),
        "bundle" | "bundled" => "bundle".to_string(),
        _ => "browse".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Realistic collection.json for testing.
    fn sample_collection_json() -> &'static str {
        r#"{
            "name": "Ultimate Skyrim SE",
            "author": "TestAuthor",
            "description": "A comprehensive overhaul for Skyrim SE",
            "game_domain": "skyrimspecialedition",
            "mods": [
                {
                    "name": "SKSE64",
                    "version": "2.2.6",
                    "optional": false,
                    "source": {
                        "type": "direct",
                        "url": "https://skse.silverlock.org/beta/skse64_2_02_06.7z",
                        "fileSize": 5242880,
                        "md5": "abc123def456"
                    },
                    "phase": 0,
                    "fileOverrides": []
                },
                {
                    "name": "SkyUI",
                    "version": "5.2",
                    "optional": false,
                    "source": {
                        "type": "nexus",
                        "modId": 12604,
                        "fileId": 35407,
                        "updatePolicy": "prefer",
                        "md5": "ff0011aabb22",
                        "fileSize": 2097152
                    },
                    "choices": {"step1": ["option_a"]},
                    "phase": 1,
                    "fileOverrides": []
                },
                {
                    "name": "USSEP",
                    "version": "4.3.0",
                    "optional": false,
                    "source": {
                        "type": "nexus",
                        "modId": 266,
                        "fileId": 412345,
                        "updatePolicy": "exact",
                        "md5": "cc33dd44ee55"
                    },
                    "phase": 1,
                    "fileOverrides": []
                },
                {
                    "name": "Enhanced Blood Textures",
                    "version": "4.0",
                    "optional": true,
                    "source": {
                        "type": "nexus",
                        "modId": 2357,
                        "fileId": 98765
                    },
                    "instructions": "Choose 'Lite' preset during FOMOD",
                    "phase": 2,
                    "fileOverrides": []
                },
                {
                    "name": "ENB Helper",
                    "version": "1.5",
                    "optional": true,
                    "source": {
                        "type": "browse",
                        "url": "https://enbdev.com/download_helper.htm",
                        "instructions": "Download the SE version manually"
                    },
                    "phase": 2,
                    "fileOverrides": []
                }
            ],
            "modRules": [
                {
                    "source": { "logicalFileName": "skyui" },
                    "type": "requires",
                    "reference": { "logicalFileName": "skse64" }
                },
                {
                    "source": { "logicalFileName": "ussep" },
                    "type": "before",
                    "reference": { "logicalFileName": "enhanced blood textures" }
                },
                {
                    "source": { "logicalFileName": "skyui" },
                    "type": "after",
                    "reference": { "logicalFileName": "ussep" }
                }
            ],
            "plugins": [
                { "name": "Skyrim.esm", "enabled": true },
                { "name": "Update.esm", "enabled": true },
                { "name": "Dawnguard.esm", "enabled": true },
                { "name": "HearthFires.esm", "enabled": true },
                { "name": "Dragonborn.esm", "enabled": true },
                { "name": "SkyUI_SE.esp", "enabled": true },
                { "name": "Unofficial Skyrim Special Edition Patch.esp", "enabled": true },
                { "name": "dD-No Twitching Dragon Death ESPFE.esp", "enabled": false }
            ]
        }"#
    }

    #[test]
    fn test_parse_collection_json() {
        let manifest = parse_collection_json(sample_collection_json()).unwrap();

        assert_eq!(manifest.name, "Ultimate Skyrim SE");
        assert_eq!(manifest.author, "TestAuthor");
        assert_eq!(manifest.game_domain, "skyrimspecialedition");
        assert_eq!(manifest.mods.len(), 5);

        let skse = &manifest.mods[0];
        assert_eq!(skse.name, "SKSE64");
        assert_eq!(skse.version, "2.2.6");
        assert!(!skse.optional);
        assert_eq!(skse.source.source_type, "direct");
        assert_eq!(
            skse.source.url.as_deref(),
            Some("https://skse.silverlock.org/beta/skse64_2_02_06.7z")
        );
        assert_eq!(skse.phase, Some(0));

        let skyui = &manifest.mods[1];
        assert_eq!(skyui.name, "SkyUI");
        assert_eq!(skyui.source.source_type, "nexus");
        assert_eq!(skyui.source.mod_id, Some(12604));
        assert_eq!(skyui.source.file_id, Some(35407));
        assert_eq!(skyui.source.update_policy.as_deref(), Some("prefer"));
        assert!(skyui.choices.is_some());

        let ebt = &manifest.mods[3];
        assert!(ebt.optional);
        assert_eq!(
            ebt.instructions.as_deref(),
            Some("Choose 'Lite' preset during FOMOD")
        );
    }

    #[test]
    fn test_mod_rule_resolution() {
        let manifest = parse_collection_json(sample_collection_json()).unwrap();
        let order = resolve_install_order(&manifest);

        assert_eq!(order.len(), manifest.mods.len());

        // Phase 0 mods (SKSE64, index 0) must come before phase 1 mods
        let skse_pos = order.iter().position(|&i| i == 0).unwrap();
        let skyui_pos = order.iter().position(|&i| i == 1).unwrap();
        let ussep_pos = order.iter().position(|&i| i == 2).unwrap();

        // SKSE64 is phase 0, SkyUI and USSEP are phase 1
        assert!(
            skse_pos < skyui_pos,
            "SKSE64 (phase 0) should be installed before SkyUI (phase 1)"
        );
        assert!(
            skse_pos < ussep_pos,
            "SKSE64 (phase 0) should be installed before USSEP (phase 1)"
        );

        // Within phase 1: SkyUI requires SKSE64 (already handled by phase),
        // and SkyUI should be after USSEP
        assert!(
            ussep_pos < skyui_pos,
            "USSEP should be installed before SkyUI (after rule)"
        );

        // Phase 2 mods come last
        let ebt_pos = order.iter().position(|&i| i == 3).unwrap();
        let enb_pos = order.iter().position(|&i| i == 4).unwrap();
        assert!(
            ebt_pos > ussep_pos,
            "EBT (phase 2) should be after USSEP (phase 1)"
        );
        assert!(
            enb_pos > skyui_pos,
            "ENB Helper (phase 2) should be after SkyUI (phase 1)"
        );
    }

    #[test]
    fn test_game_domain_display() {
        assert_eq!(game_domain_display("skyrimspecialedition"), "Skyrim SE");
        assert_eq!(game_domain_display("fallout4"), "Fallout 4");
        assert_eq!(game_domain_display("oblivion"), "Oblivion");
        assert_eq!(game_domain_display("baldursgate3"), "Baldur's Gate 3");
        assert_eq!(game_domain_display("cyberpunk2077"), "Cyberpunk 2077");
        // Unknown domains pass through unchanged
        assert_eq!(game_domain_display("somecustomgame"), "somecustomgame");
    }

    #[test]
    fn test_empty_collection_handling() {
        let json = r#"{
            "name": "Empty Collection",
            "author": "Nobody",
            "description": "",
            "game_domain": "skyrimspecialedition",
            "mods": [],
            "modRules": [],
            "plugins": []
        }"#;

        let manifest = parse_collection_json(json).unwrap();
        assert_eq!(manifest.name, "Empty Collection");
        assert_eq!(manifest.mods.len(), 0);
        assert_eq!(manifest.mod_rules.len(), 0);
        assert_eq!(manifest.plugins.len(), 0);

        let order = resolve_install_order(&manifest);
        assert!(order.is_empty());
    }

    #[test]
    fn test_source_type_classification() {
        let manifest = parse_collection_json(sample_collection_json()).unwrap();

        let nexus_mods: Vec<&CollectionModEntry> = manifest
            .mods
            .iter()
            .filter(|m| m.source.source_type == "nexus")
            .collect();
        assert_eq!(nexus_mods.len(), 3, "Should have 3 nexus source mods");

        let direct_mods: Vec<&CollectionModEntry> = manifest
            .mods
            .iter()
            .filter(|m| m.source.source_type == "direct")
            .collect();
        assert_eq!(direct_mods.len(), 1, "Should have 1 direct source mod");
        assert_eq!(direct_mods[0].name, "SKSE64");

        let browse_mods: Vec<&CollectionModEntry> = manifest
            .mods
            .iter()
            .filter(|m| m.source.source_type == "browse")
            .collect();
        assert_eq!(browse_mods.len(), 1, "Should have 1 browse source mod");
        assert_eq!(browse_mods[0].name, "ENB Helper");

        // Verify external source classifier
        assert_eq!(classify_external_source("direct"), "direct");
        assert_eq!(classify_external_source("DirectDownload"), "direct");
        assert_eq!(classify_external_source("browse"), "browse");
        assert_eq!(classify_external_source("website"), "browse");
        assert_eq!(classify_external_source("manual"), "manual");
        assert_eq!(classify_external_source("bundle"), "bundle");
        assert_eq!(classify_external_source("unknown_type"), "browse");
    }

    #[test]
    fn test_plugin_list_parsing() {
        let manifest = parse_collection_json(sample_collection_json()).unwrap();

        assert_eq!(manifest.plugins.len(), 8);

        // Verify the core ESMs are present and enabled
        let skyrim_esm = &manifest.plugins[0];
        assert_eq!(skyrim_esm.name, "Skyrim.esm");
        assert!(skyrim_esm.enabled);

        let update_esm = &manifest.plugins[1];
        assert_eq!(update_esm.name, "Update.esm");
        assert!(update_esm.enabled);

        // Check an ESP
        let skyui_esp = &manifest.plugins[5];
        assert_eq!(skyui_esp.name, "SkyUI_SE.esp");
        assert!(skyui_esp.enabled);

        // Check disabled plugin
        let disabled = &manifest.plugins[7];
        assert_eq!(disabled.name, "dD-No Twitching Dragon Death ESPFE.esp");
        assert!(!disabled.enabled);

        // Verify we can count enabled vs disabled
        let enabled_count = manifest.plugins.iter().filter(|p| p.enabled).count();
        let disabled_count = manifest.plugins.iter().filter(|p| !p.enabled).count();
        assert_eq!(enabled_count, 7);
        assert_eq!(disabled_count, 1);
    }

    #[test]
    fn test_manifest_with_patches_and_file_overrides() {
        let json = r#"{
            "name": "Patched Collection",
            "author": "Patcher",
            "description": "Tests patches and file overrides",
            "game_domain": "skyrimspecialedition",
            "mods": [
                {
                    "name": "Base Mod",
                    "version": "1.0",
                    "optional": false,
                    "source": {
                        "type": "nexus",
                        "modId": 100,
                        "fileId": 200
                    },
                    "patches": {
                        "meshes/test.nif": "patch_data_base64_here"
                    },
                    "fileOverrides": ["textures/override.dds", "meshes/replaced.nif"]
                }
            ],
            "modRules": [],
            "plugins": []
        }"#;

        let manifest = parse_collection_json(json).unwrap();
        let m = &manifest.mods[0];

        assert!(m.patches.is_some());
        let patches = m.patches.as_ref().unwrap();
        assert_eq!(patches.len(), 1);
        assert!(patches.contains_key("meshes/test.nif"));

        assert_eq!(m.file_overrides.len(), 2);
        assert_eq!(m.file_overrides[0], "textures/override.dds");
        assert_eq!(m.file_overrides[1], "meshes/replaced.nif");
    }

    #[test]
    fn test_minimal_manifest_with_defaults() {
        // Verifies serde defaults work for missing optional fields
        let json = r#"{
            "name": "Minimal",
            "author": "Min",
            "description": "Bare minimum",
            "game_domain": "fallout4",
            "mods": [
                {
                    "name": "Only Mod",
                    "source": { "type": "nexus", "modId": 1, "fileId": 2 }
                }
            ]
        }"#;

        let manifest = parse_collection_json(json).unwrap();
        assert_eq!(manifest.name, "Minimal");
        assert_eq!(manifest.game_domain, "fallout4");
        assert_eq!(manifest.mods.len(), 1);
        assert!(manifest.mod_rules.is_empty());
        assert!(manifest.plugins.is_empty());
        assert!(manifest.install_instructions.is_none());

        let m = &manifest.mods[0];
        assert_eq!(m.version, "");
        assert!(!m.optional);
        assert!(m.choices.is_none());
        assert!(m.patches.is_none());
        assert!(m.instructions.is_none());
        assert!(m.phase.is_none());
        assert!(m.file_overrides.is_empty());
    }
}
