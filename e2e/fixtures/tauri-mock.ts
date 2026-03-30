import type { Page } from '@playwright/test';

/**
 * Injects a mock `window.__TAURI_INTERNALS__` before any page JS runs.
 * This lets Playwright tests exercise the full Svelte UI with realistic
 * data without needing the Rust backend.
 *
 * Pass `overrides` to change specific command responses per-test.
 */
export async function injectTauriMock(
  page: Page,
  overrides?: Record<string, unknown>,
) {
  const overridesJson = JSON.stringify(overrides ?? {});

  await page.addInitScript(`
(function () {
  // ---- callback registry (needed by Tauri's event system) ----
  let nextId = 1;
  const callbacks = {};

  // ---- Mock data ----
  const BOTTLES = [
    { name: "CrossOver Default", path: "/Users/test/Library/Application Support/CrossOver/Bottles/Default", source: "CrossOver" },
    { name: "Steam Proton", path: "/home/test/.local/share/Steam/steamapps/compatdata/489830", source: "Proton" },
  ];

  const GAMES = [
    {
      game_id: "skyrimse", display_name: "Skyrim Special Edition",
      nexus_slug: "skyrimspecialedition",
      game_path: "/Users/test/Library/Application Support/CrossOver/Bottles/Default/drive_c/Program Files (x86)/Steam/steamapps/common/Skyrim Special Edition",
      exe_path: "/Users/test/Library/Application Support/CrossOver/Bottles/Default/drive_c/Program Files (x86)/Steam/steamapps/common/Skyrim Special Edition/SkyrimSE.exe",
      data_dir: "/Users/test/Library/Application Support/CrossOver/Bottles/Default/drive_c/Program Files (x86)/Steam/steamapps/common/Skyrim Special Edition/Data",
      bottle_name: "CrossOver Default",
      bottle_path: "/Users/test/Library/Application Support/CrossOver/Bottles/Default",
    },
    {
      game_id: "fallout4", display_name: "Fallout 4",
      nexus_slug: "fallout4",
      game_path: "/Users/test/Library/Application Support/CrossOver/Bottles/Default/drive_c/Program Files (x86)/Steam/steamapps/common/Fallout 4",
      exe_path: null,
      data_dir: "/Users/test/Library/Application Support/CrossOver/Bottles/Default/drive_c/Program Files (x86)/Steam/steamapps/common/Fallout 4/Data",
      bottle_name: "CrossOver Default",
      bottle_path: "/Users/test/Library/Application Support/CrossOver/Bottles/Default",
    },
    {
      game_id: "hogwartslegacy", display_name: "Hogwarts Legacy",
      nexus_slug: "hogwartslegacy",
      game_path: "/Users/test/Library/Application Support/CrossOver/Bottles/Default/drive_c/Program Files (x86)/Steam/steamapps/common/Hogwarts Legacy",
      exe_path: "/Users/test/Library/Application Support/CrossOver/Bottles/Default/drive_c/Program Files (x86)/Steam/steamapps/common/Hogwarts Legacy/Phoenix/Binaries/Win64/HogwartsLegacy.exe",
      data_dir: "/Users/test/Library/Application Support/CrossOver/Bottles/Default/drive_c/Program Files (x86)/Steam/steamapps/common/Hogwarts Legacy/Phoenix/Content/Paks/~mods",
      bottle_name: "CrossOver Default",
      bottle_path: "/Users/test/Library/Application Support/CrossOver/Bottles/Default",
    },
  ];

  const MODS = [
    { id: 1, game_id: "skyrimse", bottle_name: "CrossOver Default", nexus_mod_id: 12604, nexus_file_id: null, source_url: null, source_type: "nexus", name: "SkyUI", version: "5.2SE", archive_name: "SkyUI_5_2SE.7z", file_count: 14, installed_at: "2026-03-01T12:00:00Z", enabled: true, staging_path: "/tmp/staging/1", install_priority: 10, collection_name: null, user_notes: null, user_tags: [], auto_category: "UI", collection_optional: false },
    { id: 2, game_id: "skyrimse", bottle_name: "CrossOver Default", nexus_mod_id: 266, nexus_file_id: null, source_url: null, source_type: "nexus", name: "Unofficial Skyrim Special Edition Patch", version: "4.3.2", archive_name: "USSEP.7z", file_count: 87, installed_at: "2026-03-01T12:01:00Z", enabled: true, staging_path: "/tmp/staging/2", install_priority: 20, collection_name: null, user_notes: null, user_tags: ["essential"], auto_category: "Bug Fixes", collection_optional: false },
    { id: 3, game_id: "skyrimse", bottle_name: "CrossOver Default", nexus_mod_id: 272, nexus_file_id: null, source_url: null, source_type: "nexus", name: "Alternate Start - Live Another Life", version: "4.1.8", archive_name: "AlternateStart.7z", file_count: 22, installed_at: "2026-03-01T12:02:00Z", enabled: true, staging_path: "/tmp/staging/3", install_priority: 30, collection_name: null, user_notes: null, user_tags: [], auto_category: "Gameplay", collection_optional: false },
    { id: 4, game_id: "skyrimse", bottle_name: "CrossOver Default", nexus_mod_id: 32444, nexus_file_id: null, source_url: null, source_type: "nexus", name: "ENB Helper SE", version: "2.2", archive_name: "ENBHelper.7z", file_count: 3, installed_at: "2026-03-01T12:03:00Z", enabled: false, staging_path: "/tmp/staging/4", install_priority: 40, collection_name: null, user_notes: null, user_tags: [], auto_category: "Visuals", collection_optional: false },
    { id: 5, game_id: "skyrimse", bottle_name: "CrossOver Default", nexus_mod_id: 32444, nexus_file_id: null, source_url: null, source_type: "nexus", name: "Address Library for SKSE Plugins", version: "11", archive_name: "AddressLibrary.7z", file_count: 1, installed_at: "2026-03-01T12:04:00Z", enabled: true, staging_path: "/tmp/staging/5", install_priority: 5, collection_name: null, user_notes: null, user_tags: ["essential"], auto_category: "Utilities", collection_optional: false },
    { id: 6, game_id: "skyrimse", bottle_name: "CrossOver Default", nexus_mod_id: null, nexus_file_id: null, source_url: null, source_type: "manual", name: "SKSE Scripts", version: "2.2.6", archive_name: "skse64_2_02_06.7z", file_count: 45, installed_at: "2026-03-01T12:05:00Z", enabled: false, staging_path: "/tmp/staging/6", install_priority: 1, collection_name: null, user_notes: "SKSE script files", user_tags: [], auto_category: "Utilities", collection_optional: false },
    // Hogwarts Legacy mods
    { id: 7, game_id: "hogwartslegacy", bottle_name: "CrossOver Default", nexus_mod_id: 942, nexus_file_id: null, source_url: null, source_type: "nexus", name: "RE-UE4SS", version: "2.5.1", archive_name: "UE4SS_v2.5.1.zip", file_count: 8, installed_at: "2026-03-20T10:00:00Z", enabled: true, staging_path: "/tmp/staging/7", install_priority: 1, collection_name: null, user_notes: null, user_tags: ["essential"], auto_category: "Framework", collection_optional: false },
    { id: 8, game_id: "hogwartslegacy", bottle_name: "CrossOver Default", nexus_mod_id: 56, nexus_file_id: null, source_url: null, source_type: "nexus", name: "Blueprint Apparate Modloader", version: "1.0.1", archive_name: "BPApparate.zip", file_count: 3, installed_at: "2026-03-20T10:01:00Z", enabled: true, staging_path: "/tmp/staging/8", install_priority: 5, collection_name: null, user_notes: null, user_tags: ["essential"], auto_category: "Framework", collection_optional: false },
    { id: 9, game_id: "hogwartslegacy", bottle_name: "CrossOver Default", nexus_mod_id: 69, nexus_file_id: null, source_url: null, source_type: "nexus", name: "Ascendio III", version: "3.0", archive_name: "Ascendio3.zip", file_count: 1, installed_at: "2026-03-20T10:02:00Z", enabled: true, staging_path: "/tmp/staging/9", install_priority: 10, collection_name: null, user_notes: null, user_tags: [], auto_category: "Performance", collection_optional: false },
    { id: 10, game_id: "hogwartslegacy", bottle_name: "CrossOver Default", nexus_mod_id: 974, nexus_file_id: null, source_url: null, source_type: "nexus", name: "Character Editor", version: "2.0", archive_name: "CharEditor.pak", file_count: 1, installed_at: "2026-03-20T10:03:00Z", enabled: true, staging_path: "/tmp/staging/10", install_priority: 20, collection_name: "The Goblet", user_notes: null, user_tags: [], auto_category: "Cosmetics", collection_optional: false },
    { id: 11, game_id: "hogwartslegacy", bottle_name: "CrossOver Default", nexus_mod_id: 178, nexus_file_id: null, source_url: null, source_type: "nexus", name: "Hogwarts Mod Merger Output", version: "0.12.1", archive_name: "zMergedMods_P.pak", file_count: 1, installed_at: "2026-03-20T10:04:00Z", enabled: true, staging_path: "/tmp/staging/11", install_priority: 99, collection_name: "The Goblet", user_notes: "Merged PhoenixShipData.sqlite", user_tags: [], auto_category: "Utility", collection_optional: false },
  ];

  const CONFIG = {
    has_completed_setup: true,
    last_known_version: "0.9.43",
    download_dir: "/Users/test/Downloads/Corkscrew",
    disable_game_fixes: "false",
    nexus_api_key: null,
    nexus_premium: false,
  };

  const STARTERS = [
    { label: "What mods should I install first?", prompt: "I'm new to modding Skyrim. What essential mods should I install first?" },
    { label: "Check my mod list for issues", prompt: "Can you check my installed mods for any compatibility issues or missing patches?" },
    { label: "Help me fix a crash", prompt: "My game keeps crashing. Can you help me diagnose the issue?" },
  ];

  // ---- Per-test overrides ----
  const overrides = ${overridesJson};

  // ---- Command router ----
  function mockInvoke(cmd, args) {
    // Check overrides first
    if (cmd in overrides) {
      const val = overrides[cmd];
      return typeof val === "function" ? val(args) : val;
    }

    switch (cmd) {
      // Tauri plugins
      case "plugin:app|version": return "0.9.43";
      case "plugin:event|listen": {
        const id = nextId++;
        return id;
      }
      case "plugin:event|emit": return null;
      case "plugin:event|unlisten": return null;
      case "plugin:opener|open_url": return null;
      case "plugin:opener|reveal_item_in_dir": return null;
      case "plugin:deep-link|get_current": return null;
      case "plugin:updater|check": return null;
      case "plugin:updater|download_and_install": return null;

      // Layout onMount
      case "get_config": return CONFIG;
      case "get_all_games": return GAMES;
      case "get_download_queue": return [];
      case "get_notification_count": return 0;
      case "set_config_value": return null;
      case "list_profiles": return [{ name: "default", is_active: true, game_id: "skyrimse", bottle_name: "CrossOver Default" }];
      case "list_installed_collections": return [
        { slug: "uehwil", name: "The Goblet", game_domain: "hogwartslegacy", game_id: "hogwartslegacy", bottle_name: "CrossOver Default", author: "v2", latest_revision: 63, installed_revision: 63, mod_count: 132, download_size: 4294967296, status: "installed" },
      ];
      case "get_all_interrupted_installs": return [];
      case "get_pending_wabbajack_installs": return [];
      case "check_steam_status": return { installed: false, registered: false, is_deck: false };
      case "check_skyrim_version": return { current_version: "1.6.1170", target_version: "1.5.97", is_downgraded: false, downgrade_path: null };
      case "get_pinned_game_version": return "1.6.1170";
      case "chat_check_new_crashes": return { count: 0, entries: [] };

      // Dashboard
      case "get_bottles": return BOTTLES;

      // Mods page
      case "get_installed_mods_summary": {
        const gid = args && args.gameId;
        return gid ? MODS.filter(m => m.game_id === gid) : MODS;
      }
      case "is_deploy_in_progress": return false;
      case "get_game_lock_status": return null;
      case "detect_mod_tools": return [];
      case "check_skse": return { installed: false, loader_path: null, version: null, use_skse: false };
      case "get_available_disk_space": return 50000000000;
      case "get_conflicts": return [];
      case "toggle_mod": return null;
      case "get_deployment_stats": return { total_files: 172, deployed_files: 172, missing_files: 0, orphaned_files: 0 };

      // Settings
      case "get_platform_detail": return { os: "macOS", arch: "aarch64", crossover_version: null };
      case "check_deployment_health": return { status: "healthy", issues: [] };
      case "list_download_archives": return [];
      case "get_downloads_stats": return { total_size_bytes: 0, archive_count: 0, directory: "/tmp" };
      case "get_optimal_download_threads": return 6;
      case "get_verification_level": return "Balanced";
      case "vortex_list_cached_extensions": return [];

      // Chat
      case "check_ollama_status_cmd": return { installed: false, running: false, available_models: [] };
      case "check_mlx_status": return false;
      case "chat_get_state": return { model: null, backend: "Ollama", loaded: false, messages: [], available_models: [], cloud_provider: null, google_auth: null };
      case "chat_get_starters": return STARTERS;
      case "chat_get_history": return [];
      case "chat_clear_history": return null;

      // Default: return null (safe for most optional data)
      default:
        return null;
    }
  }

  // ---- Install the mock ----
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd, args) {
      window.__TAURI_MOCK_LOG__ = window.__TAURI_MOCK_LOG__ || [];
      window.__TAURI_MOCK_LOG__.push({ cmd, args, time: Date.now() });
      try {
        const result = mockInvoke(cmd, args || {});
        return Promise.resolve(result);
      } catch (e) {
        return Promise.reject(e);
      }
    },
    transformCallback: function (callback, once) {
      const id = nextId++;
      callbacks[id] = { callback, once: !!once };
      return id;
    },
    unregisterCallback: function (id) {
      delete callbacks[id];
    },
    convertFileSrc: function (path, protocol) {
      return (protocol || "asset") + "://localhost/" + path;
    },
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { windowLabel: "main", label: "main" },
    },
  };

  // Event plugin internals (prevents errors from event system)
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener: function () {},
  };

  window.__TAURI_MOCK_LOG__ = [];
})();
  `);
}
