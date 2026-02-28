export interface Bottle {
  name: string;
  path: string;
  source: string;
}

export interface DetectedGame {
  game_id: string;
  display_name: string;
  nexus_slug: string;
  game_path: string;
  exe_path: string | null;
  data_dir: string;
  bottle_name: string;
  bottle_path: string;
}

export type ModSourceType = "nexus" | "direct" | "loverslab" | "moddb" | "curseforge" | "github" | "mega" | "google_drive" | "mediafire" | "manual";

export interface InstalledMod {
  id: number;
  game_id: string;
  bottle_name: string;
  nexus_mod_id: number | null;
  nexus_file_id: number | null;
  source_url: string | null;
  source_type: ModSourceType;
  name: string;
  version: string;
  archive_name: string;
  installed_files: string[];
  installed_at: string;
  enabled: boolean;
  staging_path: string | null;
  install_priority: number;
  collection_name: string | null;
  user_notes: string | null;
  user_tags: string[];
  auto_category: string | null;
}

export interface NotificationEntry {
  id: number;
  level: string;
  message: string;
  detail: string | null;
  created_at: string;
}

export interface DeployProgress {
  current: number;
  total: number;
  mod_name: string;
  files_deployed: number;
  total_files: number;
}

export interface PluginEntry {
  filename: string;
  enabled: boolean;
}

export interface AppConfig {
  nexus_api_key: string | null;
  download_dir: string | null;
  staging_dir: string | null;
  has_completed_setup: boolean;
  controller_mode: boolean;
  [key: string]: unknown;
}

export interface LaunchResult {
  executable: string;
  bottle_name: string;
  pid: number | null;
  success: boolean;
  warning: string | null;
}

export interface SkseStatus {
  installed: boolean;
  loader_path: string | null;
  version: string | null;
  use_skse: boolean;
}

export interface DowngradeStatus {
  current_version: string;
  target_version: string;
  is_downgraded: boolean;
  downgrade_path: string | null;
}

export interface SkseCompatibility {
  compatible: boolean;
  skse_version: string | null;
  game_version: string;
  expected_game_versions: [string, string] | null;
  message: string;
  severity: "ok" | "warning" | "error";
}

export interface SkseBuild {
  tag: string;
  version: string;
  target_game_version: string;
  download_url: string;
  filename: string;
  is_recommended: boolean;
}

export interface SkseAvailableBuilds {
  game_version: string;
  edition: string;
  recommended: SkseBuild | null;
  all_builds: SkseBuild[];
}

export interface DisplaySettings {
  width: number;
  height: number;
  fullscreen: boolean;
  borderless: boolean;
}

export interface DisplayFixResult {
  fixed: boolean;
  prefs_path: string;
  previous: DisplaySettings;
  applied: DisplaySettings;
  screen_width: number;
  screen_height: number;
}

export interface ModConflict {
  file_path: string;
  existing_mod_name: string;
}

export interface DeploymentEntry {
  id: number;
  game_id: string;
  bottle_name: string;
  mod_id: number;
  relative_path: string;
  staging_path: string;
  deploy_method: string;
  sha256: string | null;
  deployed_at: string;
  mod_name: string;
}

export interface FileConflict {
  relative_path: string;
  mods: ConflictModInfo[];
  winner_mod_id: number;
}

export interface ConflictModInfo {
  mod_id: number;
  mod_name: string;
  priority: number;
}

export interface DeployResult {
  deployed_count: number;
  skipped_count: number;
  fallback_used: boolean;
}

export interface IncrementalDeployResult {
  files_added: number;
  files_removed: number;
  files_updated: number;
  files_unchanged: number;
  fallback_used: boolean;
  verification_failures: string[];
}

export type VerificationLevel = "Fast" | "Balanced" | "Paranoid";

export interface DeploymentHealth {
  // Common
  total_mods: number;
  // From check_deployment_health (settings health check)
  healthy?: boolean;
  enabled_mods?: number;
  staging_ok?: number;
  staging_missing?: number;
  staging_empty?: number;
  no_staging_path?: number;
  manifest_entries?: number;
  deployed_files_ok?: number;
  deployed_files_missing?: number;
  problem_mods?: { id: number; name: string; issue: string }[];
  needs_reinstall?: boolean;
  needs_redeploy?: boolean;
  // Verification results (Balanced/Paranoid modes)
  verification_level?: VerificationLevel;
  hash_checked?: number;
  hash_mismatches?: number;
  hash_skipped_no_record?: number;
  mismatched_files?: string[];
  // From get_deployment_health (sidebar deploy status)
  total_deployed?: number;
  total_enabled?: number;
  conflict_count?: number;
  deploy_method?: string;
  is_deployed?: boolean;
}

export type ConflictStatus = "AuthorResolved" | "Suggested" | "Manual" | "IdenticalContent";

export interface ConflictSuggestion {
  relative_path: string;
  current_winner_id: number;
  suggested_winner_id: number;
  suggested_winner_name: string;
  status: ConflictStatus;
  reason: string;
  mods: ConflictModBrief[];
}

export interface ConflictModBrief {
  mod_id: number;
  mod_name: string;
  priority: number;
  collection_name: string | null;
}

export interface ResolutionResult {
  total_conflicts: number;
  author_resolved: number;
  auto_suggested: number;
  manual_needed: number;
  priorities_changed: number;
  identical_content: number;
}

export interface IdenticalContentStats {
  fully_identical: number;
  partially_identical: number;
  identical_files_total: number;
}

export interface AnalyzeConflictsResponse {
  suggestions: ConflictSuggestion[];
  identical_stats: IdenticalContentStats;
}

export interface FomodInstaller {
  module_name: string;
  steps: FomodStep[];
  required_files: FomodFile[];
}

export interface FomodStep {
  name: string;
  groups: FomodGroup[];
}

export interface FomodGroup {
  name: string;
  group_type: string;
  options: FomodOption[];
}

export interface FomodOption {
  name: string;
  description: string;
  image: string | null;
  files: FomodFile[];
  type_descriptor: string;
}

export interface FomodFile {
  source: string;
  destination: string;
  priority: number;
  is_folder: boolean;
}

export interface SortResult {
  sorted_order: string[];
  plugins_moved: number;
  warnings: PluginWarning[];
}

export interface PluginWarning {
  plugin_name: string;
  level: "info" | "warn" | "error";
  message: string;
}

export interface CustomExecutable {
  id: number;
  game_id: string;
  bottle_name: string;
  name: string;
  exe_path: string;
  working_dir: string | null;
  args: string | null;
  is_default: boolean;
}

export interface Profile {
  id: number;
  game_id: string;
  bottle_name: string;
  name: string;
  is_active: boolean;
  created_at: string;
}

export interface ProfileSaveInfo {
  profile_id: number;
  file_count: number;
  total_size: number;
  has_backup: boolean;
}

export interface ModUpdateInfo {
  mod_id: number;
  nexus_mod_id: number;
  mod_name: string;
  current_version: string;
  latest_version: string;
  latest_file_name: string;
  latest_file_id: number;
}

export interface IntegrityReport {
  modified_files: string[];
  unknown_files: string[];
  missing_files: string[];
  total_scanned: number;
}

// Background Hashing Progress

export interface HashingProgress {
  totalFiles: number;
  hashedFiles: number;
  hashedBytes: number;
  totalBytes: number;
  modsDone: number;
  modsTotal: number;
  gameRunning: boolean;
  done: boolean;
  error: string | null;
}

// Game Directory Cleaner

export interface NonStockFile {
  relative_path: string;
  size: number;
  is_managed: boolean;
  category: string;
}

export interface CleanReport {
  non_stock_files: NonStockFile[];
  total_size: number;
  snapshot_file_count: number;
  disk_file_count: number;
  managed_count: number;
  orphaned_count: number;
  enb_files: string[];
  save_files: string[];
}

export interface CleanOptions {
  remove_loose_files: boolean;
  remove_archives: boolean;
  remove_enb: boolean;
  remove_saves: boolean;
  remove_skse: boolean;
  orphans_only: boolean;
  dry_run: boolean;
  exclude_patterns: string[];
}

export interface CleanResult {
  removed_files: string[];
  skipped_files: string[];
  bytes_freed: number;
  dry_run: boolean;
}

// DLC Detection

export interface DlcStatus {
  all_present: boolean;
  dlcs: DlcInfo[];
  game_initialized: boolean;
}

export interface DlcInfo {
  name: string;
  present: boolean;
  missing_files: string[];
}

// Wabbajack Modlists

export interface ModlistSummary {
  title: string;
  description: string;
  author: string;
  game: string;
  tags: string[];
  nsfw: boolean;
  version: string;
  image_url: string;
  readme_url: string;
  download_url: string;
  machine_url: string;
  repository: string;
  download_size: number;
  install_size: number;
  archive_count: number;
  file_count: number;
}

export interface ParsedModlist {
  name: string;
  author: string;
  description: string;
  version: string;
  game_type: number;
  game_name: string;
  is_nsfw: boolean;
  archive_count: number;
  total_download_size: number;
  directive_count: number;
  directive_breakdown: Record<string, number>;
  archives: ArchiveSummary[];
}

export interface ArchiveSummary {
  name: string;
  size: number;
  source_type: string;
  nexus_mod_id: number | null;
  nexus_file_id: number | null;
}

// OAuth

export interface TokenPair {
  access_token: string;
  refresh_token: string;
  expires_at: number;
}

export interface NexusUserInfo {
  name: string;
  email: string | null;
  avatar: string | null;
  is_premium: boolean;
  membership_roles: string[];
}

export interface AuthMethodInfo {
  type: "oauth" | "api_key" | "none";
  expires_at?: number;
  key_prefix?: string;
}

// Crash Logs

export interface CrashLogEntry {
  filename: string;
  timestamp: string;
  summary: string;
  severity: CrashSeverity;
}

export type CrashSeverity = "Critical" | "High" | "Medium" | "Low" | "Unknown";
export type Confidence = "High" | "Medium" | "Low";
export type ActionType =
  | "UpdateMod"
  | "VerifyIntegrity"
  | "SortLoadOrder"
  | "DisableMod"
  | "ReinstallMod"
  | "CheckVRAM"
  | "UpdateDrivers"
  | "CheckINI"
  | "ManualFix";

export interface CrashReport {
  log_file: string;
  timestamp: string;
  exception_type: string;
  crash_address: string;
  module_name: string;
  module_offset: string;
  diagnosis: CrashDiagnosis[];
  severity: CrashSeverity;
  involved_plugins: string[];
  involved_skse_plugins: string[];
  system_info: SystemInfo | null;
  call_stack_summary: string[];
}

export interface CrashDiagnosis {
  title: string;
  description: string;
  confidence: Confidence;
  suggested_actions: SuggestedAction[];
}

export interface SuggestedAction {
  action_type: ActionType;
  description: string;
  target: string | null;
}

export interface SystemInfo {
  os: string | null;
  cpu: string | null;
  gpu: string | null;
  ram_used_mb: number | null;
  ram_total_mb: number | null;
  vram_used_mb: number | null;
  vram_total_mb: number | null;
}

// NexusMods Browse

export interface NexusModInfo {
  mod_id: number;
  name: string;
  summary: string;
  description: string | null;
  author: string;
  category_id: number;
  version: string;
  endorsement_count: number;
  unique_downloads: number;
  picture_url: string | null;
  updated_at: string | null;
  created_at: string | null;
  available: boolean;
  adult_content: boolean;
}

export interface NexusSearchResult {
  mods: NexusModInfo[];
  total_count: number;
  offset: number;
  has_more: boolean;
}

// Endorsements

export interface EndorseResponse {
  status: string;
  message: string;
}

export interface UserEndorsement {
  mod_id: number;
  domain_name: string;
  status: string;
}

export interface NexusCategory {
  category_id: number;
  name: string;
  parent_category: number | null;
}

export interface NexusModFile {
  mod_id: number;
  file_id: number;
  name: string;
  version: string;
  file_name: string;
  size_kb: number;
  description: string;
  category: string;
}

// Collections

export interface CollectionInfo {
  slug: string;
  name: string;
  summary: string;
  description: string;
  author: string;
  game_domain: string;
  image_url: string | null;
  total_mods: number;
  endorsements: number;
  total_downloads: number;
  latest_revision: number;
  download_size: number | null;
  updated_at: string | null;
  adult_content: boolean;
  tags: string[];
}

export interface CollectionSearchResult {
  collections: CollectionInfo[];
  total_count: number;
}

export interface CollectionRevision {
  revision_number: number;
  created_at: string;
  updated_at: string | null;
  changelog: string | null;
  mod_count: number;
  download_size: number;
}

export interface CollectionMod {
  name: string;
  version: string;
  optional: boolean;
  source_type: string;
  nexus_mod_id: number | null;
  nexus_file_id: number | null;
  download_url: string | null;
  instructions: string | null;
  file_size: number | null;
  author: string | null;
  image_url: string | null;
  adult_content: boolean;
}

export interface CollectionManifest {
  name: string;
  author: string;
  description: string;
  game_domain: string;
  mods: CollectionModEntry[];
  modRules: CollectionModRule[];
  plugins: CollectionPlugin[];
  installInstructions: string | null;
}

export interface CollectionModEntry {
  name: string;
  version: string;
  optional: boolean;
  source: CollectionSource;
  choices: unknown | null;
  patches: Record<string, string> | null;
  instructions: string | null;
  phase: number | null;
  fileOverrides: string[];
  install_disabled?: boolean;
}

export interface CollectionSource {
  type: string;
  url: string | null;
  instructions: string | null;
  modId: number | null;
  fileId: number | null;
  updatePolicy: string | null;
  md5: string | null;
  fileSize: number | null;
}

export interface CollectionModRule {
  source: ModReference;
  type: string;
  reference: ModReference;
}

export interface ModReference {
  fileMD5: string | null;
  logicalFileName: string | null;
  tag: string | null;
  idHint: string | null;
}

export interface CollectionPlugin {
  name: string;
  enabled: boolean;
}

// Plugin Load Order Rules

export type PluginRuleType = "LoadAfter" | "LoadBefore" | "Group";

export interface PluginRule {
  id: number;
  game_id: string;
  bottle_name: string;
  plugin_name: string;
  rule_type: PluginRuleType;
  reference_plugin: string;
  created_at: string;
}

// Mod Rollback & Snapshots

export interface ModVersion {
  id: number;
  mod_id: number;
  version: string;
  staging_path: string;
  archive_name: string;
  created_at: string;
  is_current: boolean;
}

export interface ModSnapshot {
  id: number;
  game_id: string;
  bottle_name: string;
  name: string;
  description: string | null;
  mod_states: ModSnapshotEntry[];
  created_at: string;
}

export interface ModSnapshotEntry {
  mod_id: number;
  mod_name: string;
  version: string;
  enabled: boolean;
  priority: number;
}

// Modlist Export/Import

export interface ExportedModlist {
  format_version: number;
  app_version: string;
  exported_at: string;
  game_id: string;
  game_name: string;
  mod_count: number;
  mods: ExportedMod[];
  plugin_order: ExportedPlugin[];
  notes: string | null;
}

export interface ExportedMod {
  name: string;
  version: string;
  enabled: boolean;
  priority: number;
  nexus_mod_id: number | null;
  nexus_file_id: number | null;
  archive_name: string;
  source_url: string | null;
  source_type: string;
  installed_files: string[];
  fomod_selections: unknown | null;
}

export interface ExportedPlugin {
  filename: string;
  enabled: boolean;
}

export interface ImportPlan {
  game_id: string;
  total_mods: number;
  nexus_mods: number;
  manual_mods: number;
  already_installed: number;
  mods: ImportModStatus[];
  plugin_order: ExportedPlugin[];
}

export interface ImportModStatus {
  name: string;
  version: string;
  status: ImportStatus;
  nexus_mod_id: number | null;
  nexus_file_id: number | null;
  source_type: string;
  existing_mod_id: number | null;
}

export type ImportStatus =
  | "AlreadyInstalled"
  | "VersionMismatch"
  | "CanAutoDownload"
  | "NeedsManualDownload";

export interface ModlistDiff {
  added: string[];
  removed: string[];
  version_changed: [string, string, string][];
  priority_changed: [string, number, number][];
  enabled_changed: [string, boolean, boolean][];
}

export interface ImportResult {
  mods_updated: number;
  mods_skipped: number;
  mods_to_download: ImportDownloadAction[];
  errors: string[];
}

export interface ImportDownloadAction {
  name: string;
  version: string;
  nexus_mod_id: number | null;
  nexus_file_id: number | null;
  source_type: string;
  source_url: string | null;
}

// Bottle Settings

export interface BottleSettings {
  name: string;
  source: string;
  path: string;
  arch: string;
  windows_version: string;
  crossover_version: string;
  msync_enabled: boolean;
  metalfx_enabled: boolean;
  dxmt_nvext_enabled: boolean;
  env_vars: Record<string, string>;
  has_native_config: boolean;
}

export interface BottleSettingDef {
  key: string;
  label: string;
  description: string;
  setting_type: SettingToggle | SettingSelect | SettingReadOnly;
  recommended: string | null;
}

export interface SettingToggle {
  type: "Toggle";
  current: boolean;
}

export interface SettingSelect {
  type: "Select";
  current: string;
  options: SelectOption[];
}

export interface SettingReadOnly {
  type: "ReadOnly";
  value: string;
}

export interface SelectOption {
  value: string;
  label: string;
}

// Collection Management

export interface CollectionSummary {
  name: string;
  mod_count: number;
  enabled_count: number;
  slug: string | null;
  author: string | null;
  image_url: string | null;
  game_domain: string | null;
  installed_revision: number | null;
  original_mod_count: number | null;
}

export interface CollectionDiff {
  collection_name: string;
  installed_revision: number | null;
  latest_revision: number;
  added: DiffEntry[];
  removed: DiffEntry[];
  updated: DiffUpdate[];
  unchanged: number;
}

export interface DiffEntry {
  name: string;
  version: string;
  source_type: string;
}

export interface DiffUpdate {
  name: string;
  installed_version: string;
  latest_version: string;
  source_type: string;
}

export interface IniEdit {
  file: string;
  section: string;
  key: string;
  value: string;
  description: string;
}

export interface ModTool {
  id: string;
  name: string;
  description: string;
  exe_names: string[];
  detected_path: string | null;
  requires_wine: boolean;
  category: string;
  can_auto_install: boolean;
  github_repo: string | null;
  download_url: string | null;
  license: string;
  wine_notes: string | null;
  wine_compat: "good" | "limited" | "not_recommended";
  recommended_alternative: string | null;
  recommended_ini_edits: IniEdit[];
  support_url: string | null;
}

export interface DownloadRecord {
  id: number;
  archive_path: string;
  archive_name: string;
  nexus_mod_id: number | null;
  nexus_file_id: number | null;
  sha256: string | null;
  file_size: number;
  downloaded_at: string;
}

// Download Queue

export type DownloadStatus = "pending" | "downloading" | "completed" | "failed" | "cancelled";

export interface QueueItem {
  id: number;
  mod_name: string;
  file_name: string;
  status: DownloadStatus;
  error: string | null;
  attempt: number;
  max_attempts: number;
  downloaded_bytes: number;
  total_bytes: number;
  nexus_mod_id: number | null;
  nexus_file_id: number | null;
  url: string | null;
  game_slug: string | null;
}

export interface QueueCounts {
  pending: number;
  downloading: number;
  completed: number;
  failed: number;
  cancelled: number;
}

// Disk Budget

export interface DiskBudget {
  staging_bytes: number;
  staging_count: number;
  deployment_bytes: number;
  uses_hardlinks: boolean;
  available_bytes: number;
  game_available_bytes: number;
  total_impact_bytes: number;
}

export interface InstallImpact {
  archive_size: number;
  estimated_staging_bytes: number;
  deployment_bytes: number;
  total_bytes: number;
  uses_hardlinks: boolean;
  game_available_bytes: number;
}

// INI Manager

export interface IniFile {
  file_name: string;
  path: string;
  sections: Record<string, Record<string, string>>;
}

export interface IniSetting {
  file_name: string;
  section: string;
  key: string;
  value: string;
}

export interface IniPreset {
  name: string;
  description: string;
  settings: IniSetting[];
}

// Wine Diagnostics

export type CheckStatus = "pass" | "warning" | "error" | "skipped";

export interface DiagnosticCheck {
  name: string;
  category: string;
  status: CheckStatus;
  message: string;
  fix_available: boolean;
  fix_description: string | null;
}

export interface DiagnosticResult {
  checks: DiagnosticCheck[];
  passed: number;
  warnings: number;
  errors: number;
}

// Pre-flight

export type PreflightStatus = "pass" | "warning" | "fail";

export interface PreflightCheck {
  name: string;
  status: PreflightStatus;
  message: string;
  detail: string | null;
}

export interface PreflightResult {
  checks: PreflightCheck[];
  passed: number;
  failed: number;
  warnings: number;
  can_proceed: boolean;
}

// Mod Dependencies

export type DependencyIssueType = "missing_requirement" | "active_conflict" | "orphaned_patch";

export interface ModDependency {
  id: number;
  game_id: string;
  bottle_name: string;
  mod_id: number;
  depends_on_id: number | null;
  nexus_dep_id: number | null;
  dep_name: string;
  relationship: string;
  created_at: string;
}

export interface DependencyIssue {
  mod_id: number;
  mod_name: string;
  issue_type: DependencyIssueType;
  message: string;
  related_mod_name: string;
}

// Mod Recommendations

export interface ModRecommendation {
  nexus_mod_id: number;
  name: string;
  reason: string;
  co_occurrence_count: number;
  is_installed: boolean;
}

export interface RecommendationResult {
  mod_id: number;
  mod_name: string;
  recommendations: ModRecommendation[];
}

export interface PopularMod {
  name: string;
  nexus_mod_id: number;
  collection_count: number;
}

// Session Tracker

export interface GameSession {
  id: number;
  game_id: string;
  bottle_name: string;
  profile_name: string | null;
  started_at: string;
  ended_at: string | null;
  duration_secs: number | null;
  clean_exit: boolean | null;
  crash_log_path: string | null;
  notes: string | null;
}

export interface SessionModChange {
  id: number;
  session_id: number;
  mod_id: number | null;
  mod_name: string;
  change_type: string;
  detail: string | null;
}

export interface StabilitySummary {
  total_sessions: number;
  clean_exits: number;
  crashes: number;
  unknown_exits: number;
  avg_duration_secs: number;
  last_stable_session: string | null;
  mods_since_last_stable: SessionModChange[];
}

// FOMOD Recipes

export interface FomodRecipe {
  id: number;
  mod_id: number;
  mod_name: string;
  installer_hash: string | null;
  selections: Record<string, string[]>;
  created_at: string;
}

// Tool Requirement Detection

export interface RequiredTool {
  tool_id: string;
  tool_name: string;
  can_auto_install: boolean;
  is_detected: boolean;
  wine_compat: string;
  recommended_alternative: string | null;
  download_url: string | null;
}

export interface ToolInstallProgress {
  tool_id: string;
  phase: string;
  detail: string;
}

export interface ToolUpdateInfo {
  tool_id: string;
  tool_name: string;
  latest_version: string;
  update_available: boolean;
}

// Platform Detection

export interface PlatformInfo {
  os: string;
  is_steam_os: boolean;
  cpu_cores: number;
  cpu_brand: string;
  memory_gb: number;
  arch: string;
}

// Install Progress Events

export type InstallProgressEvent =
  | { kind: "modStarted"; mod_index: number; total_mods: number; mod_name: string }
  | { kind: "stepChanged"; mod_index: number; step: string; detail: string | null }
  | { kind: "downloadProgress"; mod_index: number; downloaded: number; total: number }
  | { kind: "modCompleted"; mod_index: number; mod_name: string; mod_id: number; deployed_size: number; duration_ms: number }
  | { kind: "modFailed"; mod_index: number; mod_name: string; error: string }
  | { kind: "collectionCompleted"; installed: number; skipped: number; failed: number }
  | { kind: "userActionRequired"; mod_index: number; mod_name: string; action: string; url: string | null; instructions: string | null }
  | { kind: "downloadPhaseStarted"; total_downloads: number; max_concurrent: number }
  | { kind: "downloadQueued"; mod_index: number; mod_name: string }
  | { kind: "downloadModStarted"; mod_index: number; mod_name: string }
  | { kind: "downloadModCompleted"; mod_index: number; mod_name: string; cached: boolean }
  | { kind: "downloadModFailed"; mod_index: number; mod_name: string; error: string }
  | { kind: "allDownloadsCompleted"; downloaded: number; cached: number; failed: number; skipped: number }
  | { kind: "downloadRetryStarted"; count: number }
  | { kind: "installPhaseStarted"; total_mods: number }
  | { kind: "stagingPhaseStarted"; total_mods: number; max_concurrent: number }
  | { kind: "stagingModStarted"; mod_index: number; mod_name: string }
  | { kind: "stagingModCompleted"; mod_index: number; mod_name: string; extracted_size?: number; duration_ms?: number }
  | { kind: "stagingModFailed"; mod_index: number; mod_name: string; error: string }
  | { kind: "stagingProgress"; mod_index: number; files_done: number; files_total: number; bytes_done: number; bytes_total: number }
  | { kind: "deployProgress"; mod_index: number; files_done: number; files_total: number; bytes_done: number; bytes_total: number }
  | { kind: "fomodRequired"; mod_index: number; mod_name: string; correlation_id: string; installer: FomodInstaller }
  | { kind: "initializing"; message: string };

// Collection Uninstall Progress Events

export type UninstallProgressEvent =
  | { kind: "uninstallStarted"; collection_name: string; total_mods: number }
  | { kind: "modUninstalling"; mod_index: number; mod_name: string; step: string }
  | { kind: "modUninstalled"; mod_index: number; mod_name: string }
  | { kind: "modUninstallFailed"; mod_index: number; mod_name: string; error: string }
  | { kind: "redeployStarted" }
  | { kind: "redeployCompleted" }
  | { kind: "uninstallCompleted"; mods_removed: number; downloads_removed: number; errors: string[] };

// Collection Install Checkpoint (for resume)
export interface CollectionInstallCheckpoint {
  id: number;
  collection_name: string;
  game_id: string;
  bottle_name: string;
  status: string;
  total_mods: number;
  completed_mods: number;
  failed_mods: number;
  skipped_mods: number;
  mod_statuses: string;
  error_message: string | null;
  created_at: string;
  updated_at: string;
}

// Wabbajack Install Progress Events (from wj-install-progress channel)
export interface WjPreflightReport {
  can_proceed: boolean;
  issues: { severity: string; message: string }[];
  total_download_size: number;
  total_archives: number;
  total_directives: number;
  cached_archives: number;
  disk_space_available: number;
  disk_space_required: number;
  nexus_archives: number;
  is_nexus_premium: boolean;
  manual_downloads: number;
}

export interface WjInstallResult {
  install_id: number;
  status: string;
  total_archives: number;
  total_directives: number;
  files_deployed: number;
  elapsed_secs: number;
  warnings: string[];
}

export type WjInstallProgressEvent =
  | { type: "PreFlightStarted" }
  | { type: "PreFlightCompleted"; report: WjPreflightReport }
  | { type: "DownloadPhaseStarted"; total: number }
  | { type: "DownloadStarted"; name: string; index: number; total: number }
  | { type: "DownloadProgress"; name: string; bytes: number; total_bytes: number }
  | { type: "DownloadCompleted"; name: string }
  | { type: "DownloadFailed"; name: string; error: string }
  | { type: "DownloadSkipped"; name: string; reason: string }
  | { type: "ExtractionStarted"; total: number; total_bytes: number }
  | { type: "ExtractionProgress"; name: string; index: number; total: number; bytes_completed: number; total_bytes: number }
  | { type: "ExtractionArchiveStarted"; name: string; index: number; total: number; size: number }
  | { type: "ExtractionArchiveCompleted"; name: string; index: number }
  | { type: "ExtractionArchiveFailed"; name: string; error: string }
  | { type: "DirectivePhaseStarted"; total: number; total_bytes: number }
  | { type: "DirectiveProgress"; current: number; total: number; directive_type: string; bytes_processed: number; total_bytes: number; current_file: string }
  | { type: "DeployStarted"; total: number; total_bytes: number; modlist_name: string }
  | { type: "DeployProgress"; current: number; total: number; bytes_deployed: number; total_bytes: number; modlist_name: string }
  | { type: "InstallCompleted"; result: WjInstallResult }
  | { type: "InstallFailed"; error: string }
  | { type: "InstallCancelled" }
  | { type: "UserActionRequired"; archive_name: string; url: string; prompt: string };

export interface WjArchiveStatus {
  name: string;
  index: number;
  size: number;
  status: "pending" | "downloading" | "downloaded" | "extracting" | "extracted" | "failed" | "skipped";
  downloadBytes?: number;
  downloadTotal?: number;
  error?: string;
}

// Wabbajack Install Status (for resume)
export interface WabbajackInstallStatus {
  install_id: number;
  modlist_name: string;
  modlist_version: string;
  status: string;
  total_archives: number;
  completed_archives: number;
  total_directives: number;
  completed_directives: number;
  error_message: string | null;
}

// Steam Integration
export interface SteamInfo {
  steam_root: string;
  userdata_dirs: string[];
  is_steam_deck: boolean;
}

export interface SteamStatus {
  installed: boolean;
  registered: boolean;
  is_deck: boolean;
}
