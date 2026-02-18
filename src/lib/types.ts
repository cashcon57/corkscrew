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
  name: string;
  version: string;
  archive_name: string;
  installed_files: string[];
  installed_at: string;
  enabled: boolean;
}

export interface PluginEntry {
  filename: string;
  enabled: boolean;
}

export interface AppConfig {
  nexus_api_key: string | null;
  download_dir: string | null;
}

export interface ModConflict {
  file_path: string;
  existing_mod_name: string;
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
