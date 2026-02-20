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
  data_dir: string;
  bottle_name: string;
  bottle_path: string;
}

export interface InstalledMod {
  id: number;
  game_id: string;
  bottle_name: string;
  nexus_mod_id: number | null;
  nexus_file_id: number | null;
  source_url: string | null;
  name: string;
  version: string;
  archive_name: string;
  installed_files: string[];
  installed_at: string;
  enabled: boolean;
  staging_path: string | null;
  install_priority: number;
}

export interface PluginEntry {
  filename: string;
  enabled: boolean;
}

export interface AppConfig {
  nexus_api_key: string | null;
  download_dir: string | null;
  [key: string]: unknown;
}

export interface LaunchResult {
  executable: string;
  bottle_name: string;
  success: boolean;
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
  mod_rules: CollectionModRule[];
  plugins: CollectionPlugin[];
  install_instructions: string | null;
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
  file_overrides: string[];
}

export interface CollectionSource {
  source_type: string;
  url: string | null;
  instructions: string | null;
  mod_id: number | null;
  file_id: number | null;
  update_policy: string | null;
  md5: string | null;
  file_size: number | null;
}

export interface CollectionModRule {
  source: ModReference;
  rule_type: string;
  reference: ModReference;
}

export interface ModReference {
  file_md5: string | null;
  logical_file_name: string | null;
  tag: string | null;
  id_hint: string | null;
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

// Install Progress Events

export type InstallProgressEvent =
  | { kind: "modStarted"; mod_index: number; total_mods: number; mod_name: string }
  | { kind: "stepChanged"; mod_index: number; step: string; detail: string | null }
  | { kind: "downloadProgress"; mod_index: number; downloaded: number; total: number }
  | { kind: "modCompleted"; mod_index: number; mod_name: string; mod_id: number }
  | { kind: "modFailed"; mod_index: number; mod_name: string; error: string }
  | { kind: "collectionCompleted"; installed: number; skipped: number; failed: number }
  | { kind: "userActionRequired"; mod_index: number; mod_name: string; action: string; url: string | null; instructions: string | null };
