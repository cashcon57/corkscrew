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
