import { invoke } from "@tauri-apps/api/core";
import type {
  Bottle,
  DetectedGame,
  InstalledMod,
  PluginEntry,
  AppConfig,
} from "./types";

// Bottles
export async function getBottles(): Promise<Bottle[]> {
  return invoke("get_bottles");
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
  modVersion?: string
): Promise<InstalledMod> {
  return invoke("install_mod_cmd", {
    archivePath,
    gameId,
    bottleName,
    modName: modName ?? null,
    modVersion: modVersion ?? "",
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
  enabled: boolean
): Promise<void> {
  return invoke("toggle_mod", { modId, enabled });
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
