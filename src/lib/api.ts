import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Bottle,
  BottleSettings,
  BottleSettingDef,
  DetectedGame,
  InstalledMod,
  PluginEntry,
  AppConfig,
  LaunchResult,
  SkseStatus,
  SkseCompatibility,
  DowngradeStatus,
  CustomExecutable,
  DeploymentEntry,
  FileConflict,
  DeployResult,
  SortResult,
  PluginWarning,
  Profile,
  ModUpdateInfo,
  IntegrityReport,
  FomodInstaller,
  FomodFile,
  ModlistSummary,
  ParsedModlist,
  TokenPair,
  NexusUserInfo,
  AuthMethodInfo,
  CrashLogEntry,
  CrashReport,
  CollectionSearchResult,
  CollectionInfo,
  CollectionRevision,
  CollectionMod,
  CollectionManifest,
  PluginRule,
  PluginRuleType,
  ModVersion,
  ModSnapshot,
  ImportPlan,
  ModlistDiff,
  DisplayFixResult,
  InstallProgressEvent,
  CollectionSummary,
  DeploymentHealth,
  QueueItem,
  QueueCounts,
  DiskBudget,
  InstallImpact,
  IniFile,
  IniPreset,
  DiagnosticResult,
  PreflightResult,
  ModDependency,
  DependencyIssue,
  RecommendationResult,
  PopularMod,
  GameSession,
  StabilitySummary,
  FomodRecipe,
  ConflictSuggestion,
  ResolutionResult,
  NotificationEntry,
  DeployProgress,
  RequiredTool,
  PlatformInfo,
  SkseAvailableBuilds,
  ImportResult,
  NexusModInfo,
} from "./types";

// Bottles
export async function getBottles(): Promise<Bottle[]> {
  return invoke("get_bottles");
}

export async function getBottleSettings(bottleName: string): Promise<BottleSettings> {
  return invoke("get_bottle_settings", { bottleName });
}

export async function getBottleSettingDefs(bottleName: string): Promise<BottleSettingDef[]> {
  return invoke("get_bottle_setting_defs", { bottleName });
}

export async function setBottleSetting(bottleName: string, key: string, value: string): Promise<void> {
  return invoke("set_bottle_setting", { bottleName, key, value });
}

// Games
export async function getGames(bottleName?: string): Promise<DetectedGame[]> {
  return invoke("get_games", { bottleName: bottleName ?? null });
}

export async function getAllGames(): Promise<DetectedGame[]> {
  return invoke("get_all_games");
}

// Mods
export async function getInstalledMods(
  gameId: string,
  bottleName: string
): Promise<InstalledMod[]> {
  return invoke("get_installed_mods", { gameId, bottleName });
}

export async function installMod(
  archivePath: string,
  gameId: string,
  bottleName: string,
  modName?: string,
  modVersion?: string,
  sourceType?: string,
  sourceUrl?: string
): Promise<InstalledMod> {
  return invoke("install_mod_cmd", {
    archivePath,
    gameId,
    bottleName,
    modName: modName ?? null,
    modVersion: modVersion ?? "",
    sourceType: sourceType ?? null,
    sourceUrl: sourceUrl ?? null,
  });
}

export async function uninstallMod(
  modId: number,
  gameId: string,
  bottleName: string
): Promise<string[]> {
  return invoke("uninstall_mod", { modId, gameId, bottleName });
}

export async function toggleMod(
  modId: number,
  gameId: string,
  bottleName: string,
  enabled: boolean
): Promise<void> {
  return invoke("toggle_mod", { modId, gameId, bottleName, enabled });
}

// Plugins (Load Order)
export async function getPluginOrder(
  gameId: string,
  bottleName: string
): Promise<PluginEntry[]> {
  return invoke("get_plugin_order", { gameId, bottleName });
}

// Nexus
export async function downloadFromNexus(
  nxmUrl: string,
  gameId: string,
  bottleName: string,
  autoInstall: boolean
): Promise<InstalledMod | string> {
  return invoke("download_from_nexus", {
    nxmUrl,
    gameId,
    bottleName,
    autoInstall,
  });
}

export async function isNexusPremium(): Promise<boolean> {
  return invoke("is_nexus_premium");
}

// Config
export async function getConfig(): Promise<AppConfig> {
  return invoke("get_config");
}

export async function setConfigValue(
  key: string,
  value: string
): Promise<void> {
  return invoke("set_config_value", { key, value });
}

export async function getGameLogo(gameId: string): Promise<string | null> {
  return invoke("get_game_logo", { gameId });
}

// Game Launching
export async function launchGame(
  gameId: string,
  bottleName: string,
  useSkse: boolean
): Promise<LaunchResult> {
  return invoke("launch_game_cmd", { gameId, bottleName, useSkse });
}

// SKSE Management
export async function checkSkse(
  gameId: string,
  bottleName: string
): Promise<SkseStatus> {
  return invoke("check_skse", { gameId, bottleName });
}

export async function getSkseDownloadUrl(): Promise<string> {
  return invoke("get_skse_download_url");
}

export async function installSkseFromArchive(
  gameId: string,
  bottleName: string,
  archivePath: string
): Promise<SkseStatus> {
  return invoke("install_skse_from_archive_cmd", { gameId, bottleName, archivePath });
}

export async function setSksePreference(
  gameId: string,
  bottleName: string,
  enabled: boolean
): Promise<void> {
  return invoke("set_skse_preference_cmd", { gameId, bottleName, enabled });
}

// SKSE Compatibility
export async function checkSkseCompatibility(
  gameId: string,
  bottleName: string
): Promise<SkseCompatibility> {
  return invoke("check_skse_compatibility_cmd", { gameId, bottleName });
}

// SKSE Auto-Download
export async function getSkseBuilds(
  gameId: string,
  bottleName: string
): Promise<SkseAvailableBuilds> {
  return invoke("get_skse_builds", { gameId, bottleName });
}

export async function installSkseAuto(
  gameId: string,
  bottleName: string
): Promise<SkseStatus> {
  return invoke("install_skse_auto_cmd", { gameId, bottleName });
}

// Display Fix
export async function fixSkyrimDisplay(
  bottleName: string
): Promise<DisplayFixResult> {
  return invoke("fix_skyrim_display", { bottleName });
}

// Skyrim Downgrade
export async function checkSkyrimVersion(
  gameId: string,
  bottleName: string
): Promise<DowngradeStatus> {
  return invoke("check_skyrim_version", { gameId, bottleName });
}

export async function downgradeSkyrim(
  gameId: string,
  bottleName: string,
  mode: string
): Promise<DowngradeStatus> {
  return invoke("downgrade_skyrim", { gameId, bottleName, mode });
}

// Vibrancy
export async function setVibrancy(material: string): Promise<void> {
  return invoke("set_vibrancy", { material });
}

// Custom Executables
export async function addCustomExe(
  gameId: string,
  bottleName: string,
  name: string,
  exePath: string,
  workingDir?: string,
  args?: string
): Promise<number> {
  return invoke("add_custom_exe", {
    gameId,
    bottleName,
    name,
    exePath,
    workingDir: workingDir ?? null,
    args: args ?? null,
  });
}

export async function removeCustomExe(exeId: number): Promise<void> {
  return invoke("remove_custom_exe", { exeId });
}

export async function listCustomExes(
  gameId: string,
  bottleName: string
): Promise<CustomExecutable[]> {
  return invoke("list_custom_exes", { gameId, bottleName });
}

export async function setDefaultExe(
  gameId: string,
  bottleName: string,
  exeId: number | null
): Promise<void> {
  return invoke("set_default_exe", { gameId, bottleName, exeId });
}

// Deployment Management
export async function getConflicts(
  gameId: string,
  bottleName: string
): Promise<FileConflict[]> {
  return invoke("get_conflicts", { gameId, bottleName });
}

export async function analyzeConflicts(
  gameId: string,
  bottleName: string
): Promise<ConflictSuggestion[]> {
  return invoke("analyze_conflicts_cmd", { gameId, bottleName });
}

export async function resolveAllConflicts(
  gameId: string,
  bottleName: string
): Promise<ResolutionResult> {
  return invoke("resolve_all_conflicts_cmd", { gameId, bottleName });
}

export async function recordConflictWinner(
  gameId: string,
  bottleName: string,
  winnerModId: number,
  loserModIds: number[]
): Promise<void> {
  return invoke("record_conflict_winner", {
    gameId,
    bottleName,
    winnerModId,
    loserModIds,
  });
}

export async function getDeploymentManifest(
  gameId: string,
  bottleName: string
): Promise<DeploymentEntry[]> {
  return invoke("get_deployment_manifest_cmd", { gameId, bottleName });
}

export async function setModPriority(
  modId: number,
  priority: number
): Promise<void> {
  return invoke("set_mod_priority", { modId, priority });
}

export async function reorderMods(
  gameId: string,
  bottleName: string,
  orderedModIds: number[]
): Promise<void> {
  return invoke("reorder_mods", { gameId, bottleName, orderedModIds });
}

export async function redeployAllMods(
  gameId: string,
  bottleName: string
): Promise<DeployResult> {
  return invoke("redeploy_all_mods", { gameId, bottleName });
}

export async function purgeDeployment(
  gameId: string,
  bottleName: string
): Promise<string[]> {
  return invoke("purge_deployment_cmd", { gameId, bottleName });
}

export async function verifyModIntegrity(modId: number): Promise<string[]> {
  return invoke("verify_mod_integrity", { modId });
}

// LOOT & Plugin Management
export async function sortPluginsLoot(
  gameId: string,
  bottleName: string
): Promise<SortResult> {
  return invoke("sort_plugins_loot", { gameId, bottleName });
}

export async function updateLootMasterlist(
  gameId: string
): Promise<string> {
  return invoke("update_loot_masterlist", { gameId });
}

export async function reorderPlugins(
  gameId: string,
  bottleName: string,
  orderedPlugins: string[]
): Promise<PluginEntry[]> {
  return invoke("reorder_plugins_cmd", { gameId, bottleName, orderedPlugins });
}

export async function togglePlugin(
  gameId: string,
  bottleName: string,
  pluginName: string,
  enabled: boolean
): Promise<PluginEntry[]> {
  return invoke("toggle_plugin_cmd", { gameId, bottleName, pluginName, enabled });
}

export async function movePlugin(
  gameId: string,
  bottleName: string,
  pluginName: string,
  newIndex: number
): Promise<PluginEntry[]> {
  return invoke("move_plugin_cmd", { gameId, bottleName, pluginName, newIndex });
}

export async function getPluginMessages(
  gameId: string,
  bottleName: string,
  pluginName: string
): Promise<PluginWarning[]> {
  return invoke("get_plugin_messages", { gameId, bottleName, pluginName });
}

// Profiles
export async function listProfiles(
  gameId: string,
  bottleName: string
): Promise<Profile[]> {
  return invoke("list_profiles_cmd", { gameId, bottleName });
}

export async function createProfile(
  gameId: string,
  bottleName: string,
  name: string
): Promise<number> {
  return invoke("create_profile_cmd", { gameId, bottleName, name });
}

export async function deleteProfile(profileId: number): Promise<void> {
  return invoke("delete_profile_cmd", { profileId });
}

export async function renameProfile(
  profileId: number,
  newName: string
): Promise<void> {
  return invoke("rename_profile_cmd", { profileId, newName });
}

export async function saveProfileSnapshot(
  profileId: number,
  gameId: string,
  bottleName: string
): Promise<void> {
  return invoke("save_profile_snapshot", { profileId, gameId, bottleName });
}

export async function activateProfile(
  profileId: number,
  gameId: string,
  bottleName: string
): Promise<void> {
  return invoke("activate_profile", { profileId, gameId, bottleName });
}

// Update Checking
export async function checkModUpdates(
  gameId: string,
  bottleName: string
): Promise<ModUpdateInfo[]> {
  return invoke("check_mod_updates", { gameId, bottleName });
}

// FOMOD
export async function detectFomod(
  stagingPath: string
): Promise<FomodInstaller | null> {
  return invoke("detect_fomod", { stagingPath });
}

export async function getFomodDefaults(
  installer: FomodInstaller
): Promise<Record<string, string[]>> {
  return invoke("get_fomod_defaults", { installer });
}

export async function getFomodFiles(
  installer: FomodInstaller,
  selections: Record<string, string[]>
): Promise<FomodFile[]> {
  return invoke("get_fomod_files", { installer, selections });
}

// Integrity
export async function createGameSnapshot(
  gameId: string,
  bottleName: string
): Promise<number> {
  return invoke("create_game_snapshot", { gameId, bottleName });
}

export async function checkGameIntegrity(
  gameId: string,
  bottleName: string
): Promise<IntegrityReport> {
  return invoke("check_game_integrity", { gameId, bottleName });
}

export async function hasGameSnapshot(
  gameId: string,
  bottleName: string
): Promise<boolean> {
  return invoke("has_game_snapshot", { gameId, bottleName });
}

// Wabbajack Modlists
export async function getWabbajackModlists(): Promise<ModlistSummary[]> {
  return invoke("get_wabbajack_modlists");
}

// Utility
export async function fetchUrlText(url: string): Promise<string> {
  return invoke("fetch_url_text", { url });
}

export async function parseWabbajackFile(
  filePath: string
): Promise<ParsedModlist> {
  return invoke("parse_wabbajack_file", { filePath });
}

export async function downloadWabbajackFile(
  url: string,
  filename: string
): Promise<string> {
  return invoke("download_wabbajack_file", { url, filename });
}

// Download Archive Management
export async function listDownloadArchives(): Promise<{
  filename: string;
  path: string;
  size_bytes: number;
  modified_at: number;
}[]> {
  return invoke("list_download_archives");
}

export async function deleteDownloadArchive(path: string): Promise<void> {
  return invoke("delete_download_archive", { path });
}

export async function getDownloadsStats(): Promise<{
  total_size_bytes: number;
  archive_count: number;
  directory: string;
}> {
  return invoke("get_downloads_stats");
}

export async function clearAllDownloadArchives(): Promise<number> {
  return invoke("clear_all_download_archives");
}

// Nexus SSO (WebSocket-based, returns API key)
export async function startNexusSso(): Promise<string> {
  return invoke("start_nexus_sso");
}

// OAuth (legacy)
export async function startNexusOAuth(
  clientId: string
): Promise<TokenPair> {
  return invoke("start_nexus_oauth", { clientId });
}

export async function refreshNexusTokens(
  clientId: string,
  refreshToken: string
): Promise<TokenPair> {
  return invoke("refresh_nexus_tokens", { clientId, refreshToken });
}

export async function saveOAuthTokens(tokens: TokenPair): Promise<void> {
  return invoke("save_oauth_tokens", { tokens });
}

export async function loadOAuthTokens(): Promise<TokenPair | null> {
  return invoke("load_oauth_tokens");
}

export async function clearOAuthTokens(): Promise<void> {
  return invoke("clear_oauth_tokens");
}

export async function getNexusUserInfo(
  accessToken: string
): Promise<NexusUserInfo> {
  return invoke("get_nexus_user_info", { accessToken });
}

export async function getAuthMethod(): Promise<AuthMethodInfo> {
  return invoke("get_auth_method_cmd");
}

export async function getNexusAccountStatus(): Promise<{
  connected: boolean;
  auth_type?: string;
  name?: string;
  email?: string | null;
  avatar?: string | null;
  is_premium?: boolean;
  membership_roles?: string[];
}> {
  return invoke("get_nexus_account_status");
}

// Crash Logs
export async function findCrashLogs(
  gameId: string,
  bottleName: string
): Promise<CrashLogEntry[]> {
  return invoke("find_crash_logs_cmd", { gameId, bottleName });
}

export async function analyzeCrashLog(
  logPath: string
): Promise<CrashReport> {
  return invoke("analyze_crash_log_cmd", { logPath });
}

// NexusMods Browse
export async function browseNexusMods(
  gameSlug: string,
  category: string,
): Promise<NexusModInfo[]> {
  return invoke("browse_nexus_mods_cmd", { gameSlug, category });
}

// NexusMods Search (GraphQL v2)
export async function searchNexusMods(
  gameSlug: string,
  searchText: string | null,
  sortBy: string | null,
  sortDir: string | null,
  count: number,
  offset: number,
  includeAdult: boolean,
): Promise<import("./types").NexusSearchResult> {
  return invoke("search_nexus_mods_cmd", {
    gameSlug,
    searchText,
    sortBy,
    sortDir,
    count,
    offset,
    includeAdult,
  });
}

export async function getGameCategories(
  gameSlug: string,
): Promise<import("./types").NexusCategory[]> {
  return invoke("get_game_categories_cmd", { gameSlug });
}

// Collections
export async function browseCollections(
  gameDomain: string,
  count: number,
  offset: number,
  sortField?: string,
  sortDirection?: string,
): Promise<CollectionSearchResult> {
  return invoke("browse_collections_cmd", {
    gameDomain, count, offset,
    sortField: sortField ?? null,
    sortDirection: sortDirection ?? null,
  });
}

export async function getCollection(
  slug: string,
  gameDomain: string
): Promise<CollectionInfo> {
  return invoke("get_collection_cmd", { slug, gameDomain });
}

export async function getCollectionRevisions(
  slug: string
): Promise<CollectionRevision[]> {
  return invoke("get_collection_revisions", { slug });
}

export async function getCollectionMods(
  slug: string,
  revision: number
): Promise<CollectionMod[]> {
  return invoke("get_collection_mods", { slug, revision });
}

export async function parseCollectionBundle(
  bundlePath: string
): Promise<CollectionManifest> {
  return invoke("parse_collection_bundle_cmd", { bundlePath });
}

export async function installCollection(
  manifest: CollectionManifest,
  gameId: string,
  bottleName: string
): Promise<{
  installed: number;
  already_installed: number;
  skipped: number;
  failed: number;
  details: { name: string; status: string; error: string | null; url: string | null; instructions: string | null }[];
}> {
  return invoke("install_collection_cmd", { manifest, gameId, bottleName });
}

// Plugin Load Order Rules
export async function addPluginRule(
  gameId: string,
  bottleName: string,
  pluginName: string,
  ruleType: PluginRuleType,
  referencePlugin: string
): Promise<number> {
  return invoke("add_plugin_rule", {
    gameId, bottleName, pluginName, ruleType, referencePlugin,
  });
}

export async function removePluginRule(ruleId: number): Promise<void> {
  return invoke("remove_plugin_rule", { ruleId });
}

export async function listPluginRules(
  gameId: string,
  bottleName: string
): Promise<PluginRule[]> {
  return invoke("list_plugin_rules", { gameId, bottleName });
}

export async function clearPluginRules(
  gameId: string,
  bottleName: string
): Promise<void> {
  return invoke("clear_plugin_rules", { gameId, bottleName });
}

// Mod Rollback & Snapshots
export async function saveModVersion(
  modId: number,
  version: string,
  stagingPath: string,
  archiveName: string
): Promise<number> {
  return invoke("save_mod_version_cmd", {
    modId, version, stagingPath, archiveName,
  });
}

export async function listModVersions(
  modId: number
): Promise<ModVersion[]> {
  return invoke("list_mod_versions_cmd", { modId });
}

export async function rollbackModVersion(
  modId: number,
  versionId: number
): Promise<ModVersion> {
  return invoke("rollback_mod_version", { modId, versionId });
}

export async function cleanupModVersions(
  modId: number,
  keepCount: number
): Promise<number> {
  return invoke("cleanup_mod_versions", { modId, keepCount });
}

export async function createModSnapshot(
  gameId: string,
  bottleName: string,
  name: string,
  description?: string
): Promise<number> {
  return invoke("create_mod_snapshot", {
    gameId, bottleName, name, description: description ?? null,
  });
}

export async function listModSnapshots(
  gameId: string,
  bottleName: string
): Promise<ModSnapshot[]> {
  return invoke("list_mod_snapshots", { gameId, bottleName });
}

export async function deleteModSnapshot(
  snapshotId: number
): Promise<void> {
  return invoke("delete_mod_snapshot", { snapshotId });
}

// Modlist Export/Import
export async function exportModlist(
  gameId: string,
  bottleName: string,
  outputPath: string,
  notes?: string
): Promise<string> {
  return invoke("export_modlist_cmd", {
    gameId, bottleName, outputPath, notes: notes ?? null,
  });
}

export async function importModlistPlan(
  filePath: string,
  gameId: string,
  bottleName: string
): Promise<ImportPlan> {
  return invoke("import_modlist_plan", { filePath, gameId, bottleName });
}

export async function diffModlists(
  filePath: string,
  gameId: string,
  bottleName: string
): Promise<ModlistDiff> {
  return invoke("diff_modlists_cmd", { filePath, gameId, bottleName });
}

export async function executeModlistImport(
  filePath: string,
  gameId: string,
  bottleName: string
): Promise<ImportResult> {
  return invoke("execute_modlist_import", { filePath, gameId, bottleName });
}

// Collection Management
export async function listInstalledCollections(
  gameId: string,
  bottleName: string
): Promise<CollectionSummary[]> {
  return invoke("list_installed_collections_cmd", { gameId, bottleName });
}

export async function setModCollectionName(
  modId: number,
  collectionName: string
): Promise<void> {
  return invoke("set_mod_collection_name_cmd", { modId, collectionName });
}

export async function switchCollection(
  gameId: string,
  bottleName: string,
  collectionName: string
): Promise<{ deployed_count: number; active_collection: string }> {
  return invoke("switch_collection_cmd", {
    gameId,
    bottleName,
    collectionName,
  });
}

export async function deleteCollection(
  gameId: string,
  bottleName: string,
  collectionName: string,
  deleteUniqueDownloads: boolean
): Promise<{ mods_removed: number; downloads_removed: number }> {
  return invoke("delete_collection_cmd", {
    gameId,
    bottleName,
    collectionName,
    deleteUniqueDownloads,
  });
}

export async function getCollectionDiff(
  gameId: string,
  bottleName: string,
  collectionName: string
): Promise<import("./types").CollectionDiff> {
  return invoke("get_collection_diff_cmd", { gameId, bottleName, collectionName });
}

export async function getDeploymentHealth(
  gameId: string,
  bottleName: string
): Promise<DeploymentHealth> {
  return invoke("get_deployment_health", { gameId, bottleName });
}

// Mod Tools
export async function detectModTools(
  gameId: string,
  bottleName: string
): Promise<import("./types").ModTool[]> {
  return invoke("detect_mod_tools_cmd", { gameId, bottleName });
}

export async function installModTool(
  toolId: string,
  gameId: string,
  bottleName: string
): Promise<string> {
  return invoke("install_mod_tool", { toolId, gameId, bottleName });
}

export async function uninstallModTool(
  toolId: string,
  gameId: string,
  bottleName: string,
  detectedPath?: string | null,
): Promise<void> {
  return invoke("uninstall_mod_tool", { toolId, gameId, bottleName, detectedPath: detectedPath ?? null });
}

export async function launchModTool(
  toolId: string,
  gameId: string,
  bottleName: string
): Promise<LaunchResult> {
  return invoke("launch_mod_tool", { toolId, gameId, bottleName });
}

export async function reinstallModTool(
  toolId: string,
  gameId: string,
  bottleName: string
): Promise<string> {
  return invoke("reinstall_mod_tool", { toolId, gameId, bottleName });
}

export async function applyToolIniEdits(
  toolId: string,
  gameId: string,
  bottleName: string
): Promise<number> {
  return invoke("apply_tool_ini_edits_cmd", { toolId, gameId, bottleName });
}

// Notes & Tags
export async function setModNotes(
  modId: number,
  notes: string | null
): Promise<void> {
  return invoke("set_mod_notes", { modId, notes });
}

export async function setModSource(
  modId: number,
  sourceType: string,
  sourceUrl: string | null
): Promise<void> {
  return invoke("set_mod_source", { modId, sourceType, sourceUrl });
}

export async function setModTags(
  modId: number,
  tags: string[]
): Promise<void> {
  return invoke("set_mod_tags", { modId, tags });
}

export async function getAllTags(
  gameId: string,
  bottleName: string
): Promise<string[]> {
  return invoke("get_all_tags", { gameId, bottleName });
}

// Download Queue
export async function getDownloadQueue(): Promise<QueueItem[]> {
  return invoke("get_download_queue");
}

export async function getDownloadQueueCounts(): Promise<QueueCounts> {
  return invoke("get_download_queue_counts");
}

export async function retryDownload(id: number): Promise<boolean> {
  return invoke("retry_download", { id });
}

export async function cancelDownload(id: number): Promise<void> {
  return invoke("cancel_download", { id });
}

export async function clearFinishedDownloads(): Promise<number> {
  return invoke("clear_finished_downloads");
}

export function onDownloadQueueUpdate(
  callback: (items: QueueItem[]) => void
): Promise<UnlistenFn> {
  return listen<QueueItem[]>("download-queue-update", (e) =>
    callback(e.payload)
  );
}

// Install Progress Events
export function onInstallProgress(
  callback: (event: InstallProgressEvent) => void
): Promise<UnlistenFn> {
  return listen<InstallProgressEvent>("install-progress", (e) =>
    callback(e.payload)
  );
}

// Disk Budget
export async function getDiskBudget(
  gameId: string,
  bottleName: string
): Promise<DiskBudget> {
  return invoke("get_disk_budget", { gameId, bottleName });
}

export async function getAvailableDiskSpace(path: string): Promise<number> {
  return invoke("get_available_disk_space_cmd", { path });
}

export async function estimateInstallImpact(
  archiveSize: number,
  gameId: string,
  bottleName: string
): Promise<InstallImpact> {
  return invoke("estimate_install_impact_cmd", { archiveSize, gameId, bottleName });
}

// INI Manager
export async function getIniSettings(
  gameId: string,
  bottleName: string
): Promise<IniFile[]> {
  return invoke("get_ini_settings", { gameId, bottleName });
}

export async function setIniSetting(
  filePath: string,
  section: string,
  key: string,
  value: string
): Promise<void> {
  return invoke("set_ini_setting", { filePath, section, key, value });
}

export async function getIniPresets(gameId: string): Promise<IniPreset[]> {
  return invoke("get_ini_presets", { gameId });
}

export async function applyIniPreset(
  gameId: string,
  bottleName: string,
  presetName: string
): Promise<number> {
  return invoke("apply_ini_preset", { gameId, bottleName, presetName });
}

// Wine Diagnostics
export async function runWineDiagnostics(
  gameId: string,
  bottleName: string
): Promise<DiagnosticResult> {
  return invoke("run_wine_diagnostics", { gameId, bottleName });
}

export async function fixWineAppdata(bottleName: string): Promise<void> {
  return invoke("fix_wine_appdata", { bottleName });
}

export async function fixWineDllOverride(
  bottleName: string,
  dllName: string,
  overrideType: string
): Promise<void> {
  return invoke("fix_wine_dll_override", { bottleName, dllName, overrideType });
}

export async function fixWineRetinaMode(bottleName: string): Promise<void> {
  return invoke("fix_wine_retina_mode", { bottleName });
}

// Pre-flight
export async function runPreflightCheck(
  gameId: string,
  bottleName: string
): Promise<PreflightResult> {
  return invoke("run_preflight_check", { gameId, bottleName });
}

// Mod Dependencies
export async function addModDependency(
  gameId: string,
  bottleName: string,
  modId: number,
  dependsOnId: number | null,
  nexusDepId: number | null,
  depName: string,
  relationship: string
): Promise<number> {
  return invoke("add_mod_dependency", {
    gameId, bottleName, modId, dependsOnId, nexusDepId, depName, relationship,
  });
}

export async function removeModDependency(depId: number): Promise<void> {
  return invoke("remove_mod_dependency", { depId });
}

export async function getModDependencies(modId: number): Promise<ModDependency[]> {
  return invoke("get_mod_dependencies", { modId });
}

export async function checkDependencyIssues(
  gameId: string,
  bottleName: string
): Promise<DependencyIssue[]> {
  return invoke("check_dependency_issues", { gameId, bottleName });
}

// Mod Recommendations
export async function getModRecommendations(
  gameId: string,
  bottleName: string,
  targetModId: number
): Promise<RecommendationResult> {
  return invoke("get_mod_recommendations", { gameId, bottleName, targetModId });
}

export async function getPopularMods(
  gameId: string,
  bottleName: string
): Promise<PopularMod[]> {
  return invoke<[string, number, number][]>("get_popular_mods", { gameId, bottleName }).then(
    (items) => items.map(([name, nexus_mod_id, collection_count]) => ({
      name, nexus_mod_id, collection_count,
    }))
  );
}

// Session Tracker
export async function startGameSession(
  gameId: string,
  bottleName: string,
  profileName?: string
): Promise<number> {
  return invoke("start_game_session", {
    gameId, bottleName, profileName: profileName ?? null,
  });
}

export async function endGameSession(
  sessionId: number,
  cleanExit: boolean,
  crashLogPath?: string
): Promise<void> {
  return invoke("end_game_session", {
    sessionId, cleanExit, crashLogPath: crashLogPath ?? null,
  });
}

export async function recordSessionModChange(
  sessionId: number,
  modId: number | null,
  modName: string,
  changeType: string,
  detail?: string
): Promise<number> {
  return invoke("record_session_mod_change", {
    sessionId, modId, modName, changeType, detail: detail ?? null,
  });
}

export async function getSessionHistory(
  gameId: string,
  bottleName: string,
  limit?: number
): Promise<GameSession[]> {
  return invoke("get_session_history", {
    gameId, bottleName, limit: limit ?? null,
  });
}

export async function getStabilitySummary(
  gameId: string,
  bottleName: string
): Promise<StabilitySummary> {
  return invoke("get_stability_summary", { gameId, bottleName });
}

// FOMOD Recipes
export async function saveFomodRecipe(
  modId: number,
  modName: string,
  installerHash: string | null,
  selections: Record<string, string[]>
): Promise<number> {
  return invoke("save_fomod_recipe", { modId, modName, installerHash, selections });
}

export async function getFomodRecipe(modId: number): Promise<FomodRecipe | null> {
  return invoke("get_fomod_recipe", { modId });
}

export async function listFomodRecipes(
  gameId: string,
  bottleName: string
): Promise<FomodRecipe[]> {
  return invoke("list_fomod_recipes", { gameId, bottleName });
}

export async function deleteFomodRecipe(modId: number): Promise<void> {
  return invoke("delete_fomod_recipe", { modId });
}

export async function hasCompatibleFomodRecipe(
  modId: number,
  currentHash?: string
): Promise<boolean> {
  return invoke("has_compatible_fomod_recipe", {
    modId, currentHash: currentHash ?? null,
  });
}

// Auto-categories
export async function backfillCategories(
  gameId: string,
  bottleName: string
): Promise<number> {
  return invoke("backfill_categories", { gameId, bottleName });
}

// Notification log
export async function getNotificationLog(limit: number = 50): Promise<NotificationEntry[]> {
  return invoke("get_notification_log", { limit });
}

export async function clearNotificationLog(): Promise<void> {
  return invoke("clear_notification_log");
}

export async function logNotification(
  level: string,
  message: string,
  detail?: string
): Promise<void> {
  return invoke("log_notification", { level, message, detail: detail ?? null });
}

export async function getNotificationCount(): Promise<number> {
  return invoke("get_notification_count");
}

// Deploy progress events
export function onDeployProgress(
  callback: (progress: DeployProgress) => void
): Promise<UnlistenFn> {
  return listen<DeployProgress>("deploy-progress", (e) =>
    callback(e.payload)
  );
}

// Tool Requirement Detection
export async function detectCollectionTools(
  manifestJson: string,
  gameId: string,
  bottleName: string
): Promise<RequiredTool[]> {
  return invoke("detect_collection_tools", { manifestJson, gameId, bottleName });
}

export async function detectWabbajackTools(
  wjPath: string,
  gameId: string,
  bottleName: string
): Promise<RequiredTool[]> {
  return invoke("detect_wabbajack_tools", { wjPath, gameId, bottleName });
}

// Platform Detection
export async function getPlatformDetail(): Promise<PlatformInfo> {
  return invoke("get_platform_detail");
}
