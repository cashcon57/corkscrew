//! CLI commands: diagnostic tools, e2e test support, shader scans, and headless launch.

use crate::config;
use crate::database;
use crate::database::ModDatabase;
use crate::display_fix;
use crate::games;
use crate::launcher;
use crate::plugins;
use crate::resolve_game;
use crate::shader_conversion;
use crate::skse;
use crate::vortex_fetcher;
use crate::vortex_runtime;
use std::path::PathBuf;
use std::sync::Arc;

// --- CLI diagnostic tools ---

/// List all installed mods for a game+bottle.
pub fn cli_list_mods(game_id: &str, bottle_name: &str, db: &Arc<ModDatabase>) {
    let mods = match db.list_mods(game_id, bottle_name) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[corkscrew] ERROR: {}", e);
            std::process::exit(1);
        }
    };
    println!(
        "[corkscrew] {} mods installed for {}:{}",
        mods.len(),
        game_id,
        bottle_name
    );
    println!(
        "{:<8} {:<50} {:<10} {:<10} Staging",
        "ID", "Name", "Enabled", "Files"
    );
    println!("{}", "-".repeat(120));
    for m in &mods {
        let staging = m.staging_path.as_deref().unwrap_or("(inline)");
        println!(
            "{:<8} {:<50} {:<10} {:<10} {}",
            m.id,
            if m.name.len() > 48 {
                format!("{}…", &m.name[..47])
            } else {
                m.name.clone()
            },
            if m.enabled { "yes" } else { "NO" },
            m.installed_files.len(),
            staging,
        );
    }
}

/// Search installed mods by name (case-insensitive substring).
pub fn cli_search_mods(query: &str, game_id: &str, bottle_name: &str, db: &Arc<ModDatabase>) {
    let mods = match db.list_mods(game_id, bottle_name) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[corkscrew] ERROR: {}", e);
            std::process::exit(1);
        }
    };
    let q = query.to_lowercase();
    let matches: Vec<_> = mods
        .iter()
        .filter(|m| m.name.to_lowercase().contains(&q))
        .collect();
    println!(
        "[corkscrew] {} match(es) for '{}' in {}:{}",
        matches.len(),
        query,
        game_id,
        bottle_name
    );
    for m in matches {
        let staging = m.staging_path.as_deref().unwrap_or("(inline)");
        println!(
            "  ID={} name='{}' enabled={} files={} nexus_id={:?}",
            m.id,
            m.name,
            m.enabled,
            m.installed_files.len(),
            m.nexus_mod_id
        );
        println!("    staging: {}", staging);
        if !m.installed_files.is_empty() {
            let plugins: Vec<_> = m
                .installed_files
                .iter()
                .filter(|f| {
                    let fl = f.to_lowercase();
                    fl.ends_with(".esp") || fl.ends_with(".esm") || fl.ends_with(".esl")
                })
                .collect();
            if !plugins.is_empty() {
                println!(
                    "    plugin files: {}",
                    plugins
                        .iter()
                        .map(|p| p.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
    }
}

/// Find files matching a pattern across all staging dirs for a game+bottle.
pub fn cli_find_file(pattern: &str, game_id: &str, bottle_name: &str, db: &Arc<ModDatabase>) {
    let mods = match db.list_mods(game_id, bottle_name) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[corkscrew] ERROR: {}", e);
            std::process::exit(1);
        }
    };

    // Also load current plugins state so we can flag deployed-but-inactive plugins
    let plugin_active: std::collections::HashMap<String, bool> = {
        // Try to get game resolution for plugins.txt
        match resolve_game(game_id, bottle_name) {
            Ok((bottle, game, _)) => {
                let game_path = PathBuf::from(&game.game_path);
                let pf = games::with_plugin(game_id, |p| p.get_plugins_file(&game_path, &bottle))
                    .flatten();
                if let Some(pf) = pf {
                    plugins::skyrim_plugins::read_plugins_txt(&pf)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|e| (e.filename.to_lowercase(), e.enabled))
                        .collect()
                } else {
                    Default::default()
                }
            }
            Err(_) => Default::default(),
        }
    };

    let pat = pattern.to_lowercase();
    let mut found = 0usize;

    for m in &mods {
        // Check registered installed_files list
        let file_matches: Vec<_> = m
            .installed_files
            .iter()
            .filter(|f| f.to_lowercase().contains(&pat))
            .collect();

        if !file_matches.is_empty() {
            println!("  [mod {}] id={} enabled={}", m.name, m.id, m.enabled);
            for f in &file_matches {
                let fl = f.to_lowercase();
                let is_plugin =
                    fl.ends_with(".esp") || fl.ends_with(".esm") || fl.ends_with(".esl");
                let basename = f.rsplit(['/', '\\']).next().unwrap_or(f.as_str());
                let active_note = if is_plugin {
                    match plugin_active.get(&basename.to_lowercase()) {
                        Some(true) => " [plugin: ACTIVE ✓]",
                        Some(false) => " [plugin: INACTIVE ✗]",
                        None => " [plugin: not in plugins.txt]",
                    }
                } else {
                    ""
                };
                println!("    {}{}", f, active_note);
                found += 1;
            }
        }

        // Also walk staging dir if available (catches files not in DB list)
        if let Some(ref sp) = m.staging_path {
            let staging = PathBuf::from(sp);
            if staging.is_dir() {
                for entry in walkdir::WalkDir::new(&staging).into_iter().flatten() {
                    let name = entry.file_name().to_string_lossy().to_lowercase();
                    if name.contains(&pat) {
                        let rel = entry.path().strip_prefix(&staging).unwrap_or(entry.path());
                        // Only show if NOT already in installed_files
                        if !m
                            .installed_files
                            .iter()
                            .any(|f| f.to_lowercase().contains(&pat))
                        {
                            if found == 0 {
                                println!("  [mod {}] id={} (staged, not deployed)", m.name, m.id);
                            }
                            println!("    staging/{}", rel.display());
                            found += 1;
                        }
                    }
                }
            }
        }
    }

    println!("[corkscrew] find-file '{}': {} result(s)", pattern, found);
}

/// Show plugin load order state: active/inactive/on-disk/stale.
pub fn cli_check_plugins(
    game_id: &str,
    bottle_name: &str,
    inactive_only: bool,
    deployed_inactive_only: bool,
    db: &Arc<ModDatabase>,
) {
    let (bottle, game, data_dir) = match resolve_game(game_id, bottle_name) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[corkscrew] ERROR: {}", e);
            std::process::exit(1);
        }
    };

    let game_path = PathBuf::from(&game.game_path);
    let pf =
        match games::with_plugin(game_id, |p| p.get_plugins_file(&game_path, &bottle)).flatten() {
            Some(p) => p,
            None => {
                eprintln!("[corkscrew] No plugins.txt path for game '{}'", game_id);
                std::process::exit(1);
            }
        };

    println!("[corkscrew] plugins.txt: {}", pf.display());
    if let Ok(meta) = std::fs::metadata(&pf) {
        if let Ok(modified) = meta.modified() {
            let secs = modified
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            println!(
                "[corkscrew] plugins.txt last modified: {} (unix {})",
                {
                    let dt = chrono::DateTime::<chrono::Local>::from(modified);
                    dt.format("%Y-%m-%d %H:%M:%S").to_string()
                },
                secs
            );
        }
    }

    // Read plugins.txt
    let entries = match plugins::skyrim_plugins::read_plugins_txt(&pf) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[corkscrew] ERROR reading plugins.txt: {}", e);
            std::process::exit(1);
        }
    };

    // Discover on-disk plugins
    let on_disk = plugins::skyrim_plugins::discover_plugins(&data_dir).unwrap_or_default();
    let on_disk_lower: std::collections::HashSet<String> =
        on_disk.iter().map(|s| s.to_lowercase()).collect();

    // Build active set from plugins.txt
    let in_txt_active: std::collections::HashMap<String, bool> = entries
        .iter()
        .map(|e| (e.filename.to_lowercase(), e.enabled))
        .collect();

    // Build "which mod owns this plugin" map from DB
    let mods = db.list_mods(game_id, bottle_name).unwrap_or_default();
    let mut plugin_owner: std::collections::HashMap<String, String> = Default::default();
    for m in &mods {
        for f in &m.installed_files {
            let fl = f.to_lowercase();
            if fl.ends_with(".esp") || fl.ends_with(".esm") || fl.ends_with(".esl") {
                let basename = f.rsplit(['/', '\\']).next().unwrap_or(f.as_str());
                plugin_owner.insert(basename.to_lowercase(), m.name.clone());
            }
        }
    }

    let active_count = entries.iter().filter(|e| e.enabled).count();
    let inactive_count = entries.iter().filter(|e| !e.enabled).count();
    println!(
        "[corkscrew] plugins.txt: {} active, {} inactive, {} total entries",
        active_count,
        inactive_count,
        entries.len()
    );
    println!("[corkscrew] on disk: {} plugin files", on_disk.len());

    // Find deployed-but-inactive: on disk AND in plugins.txt but NOT active
    let mut deployed_inactive: Vec<&str> = Vec::new();
    for p in &on_disk {
        let key = p.to_lowercase();
        if let Some(&active) = in_txt_active.get(&key) {
            if !active {
                deployed_inactive.push(p.as_str());
            }
        }
    }

    // Find on-disk but not in plugins.txt at all
    let not_in_txt: Vec<&str> = on_disk
        .iter()
        .filter(|p| !in_txt_active.contains_key(&p.to_lowercase()))
        .map(|p| p.as_str())
        .collect();

    // Find in plugins.txt but not on disk (stale)
    let stale: Vec<_> = entries
        .iter()
        .filter(|e| !on_disk_lower.contains(&e.filename.to_lowercase()))
        .collect();

    if deployed_inactive_only {
        println!(
            "\n[DEPLOYED BUT INACTIVE in plugins.txt] ({} plugins):",
            deployed_inactive.len()
        );
        for p in &deployed_inactive {
            let owner = plugin_owner
                .get(&p.to_lowercase())
                .map(|s| s.as_str())
                .unwrap_or("unknown mod");
            println!("  {} ({})", p, owner);
        }
        return;
    }

    println!(
        "\n[DEPLOYED BUT INACTIVE in plugins.txt] ({} plugins):",
        deployed_inactive.len()
    );
    for p in &deployed_inactive {
        let owner = plugin_owner
            .get(&p.to_lowercase())
            .map(|s| s.as_str())
            .unwrap_or("unknown mod");
        println!("  {} ({})", p, owner);
    }

    println!(
        "\n[ON DISK BUT NOT IN plugins.txt] ({} plugins):",
        not_in_txt.len()
    );
    for p in not_in_txt.iter().take(20) {
        println!("  {}", p);
    }
    if not_in_txt.len() > 20 {
        println!("  ... and {} more", not_in_txt.len() - 20);
    }

    println!(
        "\n[STALE: in plugins.txt but NOT on disk] ({} plugins):",
        stale.len()
    );
    for e in stale.iter().take(20) {
        println!(
            "  {} ({})",
            e.filename,
            if e.enabled { "active" } else { "inactive" }
        );
    }
    if stale.len() > 20 {
        println!("  ... and {} more", stale.len() - 20);
    }

    if !inactive_only {
        println!(
            "\n[ALL INACTIVE in plugins.txt] ({} plugins, showing first 50):",
            inactive_count
        );
        let mut shown = 0;
        for e in entries.iter().filter(|e| !e.enabled) {
            let on_disk_flag = if on_disk_lower.contains(&e.filename.to_lowercase()) {
                " [on-disk]"
            } else {
                " [missing]"
            };
            let owner = plugin_owner
                .get(&e.filename.to_lowercase())
                .map(|s| format!(" ({})", s))
                .unwrap_or_default();
            println!("  {}{}{}", e.filename, on_disk_flag, owner);
            shown += 1;
            if shown >= 50 {
                println!("  ... and {} more inactive", inactive_count - shown);
                break;
            }
        }
    }
}

/// Manually run plugin sync for a game+bottle.
pub fn cli_sync_plugins(game_id: &str, bottle_name: &str) {
    let (bottle, game, _) = match resolve_game(game_id, bottle_name) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[corkscrew] ERROR: {}", e);
            std::process::exit(1);
        }
    };
    match crate::sync_plugins_for_game(&game, &bottle) {
        Ok(()) => println!(
            "[corkscrew] Plugin sync complete for {}:{}",
            game_id, bottle_name
        ),
        Err(e) => {
            eprintln!("[corkscrew] ERROR: sync failed: {}", e);
            std::process::exit(1);
        }
    }
}

/// Show all files registered for a specific mod (by ID or name substring).
pub fn cli_mod_files(search: &str, game_id: &str, bottle_name: &str, db: &Arc<ModDatabase>) {
    let mods = match db.list_mods(game_id, bottle_name) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[corkscrew] ERROR: {}", e);
            std::process::exit(1);
        }
    };
    let q = search.to_lowercase();
    let matched: Vec<_> = if let Ok(id) = search.parse::<i64>() {
        mods.iter().filter(|m| m.id == id).collect()
    } else {
        mods.iter()
            .filter(|m| m.name.to_lowercase().contains(&q))
            .collect()
    };
    if matched.is_empty() {
        println!("[corkscrew] No mods found matching '{}'", search);
        return;
    }
    for m in matched {
        println!(
            "[mod {}] id={} enabled={} nexus_id={:?}",
            m.name, m.id, m.enabled, m.nexus_mod_id
        );
        if let Some(sp) = &m.staging_path {
            println!("  staging: {}", sp);
        }
        println!("  registered files ({}):", m.installed_files.len());
        for f in &m.installed_files {
            println!("    {}", f);
        }
        // Also list staging dir if present and different
        if let Some(sp) = &m.staging_path {
            let staging = PathBuf::from(sp);
            if staging.is_dir() {
                let staged: Vec<_> = walkdir::WalkDir::new(&staging)
                    .into_iter()
                    .flatten()
                    .filter(|e| e.file_type().is_file())
                    .map(|e| {
                        e.path()
                            .strip_prefix(&staging)
                            .unwrap_or(e.path())
                            .to_path_buf()
                    })
                    .collect();
                println!("  staged files ({}):", staged.len());
                for f in &staged {
                    println!("    {}", f.display());
                }
            }
        }
    }
}

// --- CLI e2e test support commands ---

/// List detected bottles as JSON.
pub fn cli_list_bottles() {
    let bottles = crate::bottles::detect_bottles();
    let json: Vec<serde_json::Value> = bottles
        .iter()
        .map(|b| {
            serde_json::json!({
                "name": b.name,
                "path": b.path.display().to_string(),
                "engine": &b.source,
                "exists": b.exists(),
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&json).unwrap_or_default()
    );
}

/// Diagnostic: dump everything Corkscrew detects in one bottle as plain text.
///
/// Read-only. Designed to be piped into a Discord paste so users can show
/// what Corkscrew sees without screenshots. Surfaces:
/// - bottle path + engine
/// - detected games via plugin registry (Steam appmanifest + plugins)
/// - CrossOver `.lnk` shortcuts found in the bottle
/// - per-game tool detection (ME2, SKSE, etc.)
pub fn cli_scan_bottle(bottle_name: &str) {
    let bottle = match crate::bottles::find_bottle_by_name(bottle_name) {
        Some(b) => b,
        None => {
            eprintln!("Error: Bottle '{}' not found", bottle_name);
            eprintln!("Run `corkscrew --list-bottles` to see available bottles.");
            std::process::exit(1);
        }
    };

    // -----------------------------------------------------------------------
    // Header section
    // -----------------------------------------------------------------------
    let is_sandboxed = {
        let p = bottle.path.to_string_lossy();
        p.contains("Containers/com.codeweavers.CrossOver")
    };

    println!("=== Corkscrew Bottle Diagnostic ===");
    println!("Corkscrew version : {}", env!("CARGO_PKG_VERSION"));
    println!("Bottle name       : {}", bottle.name);
    println!("Bottle path       : {}", bottle.path.display());
    println!("Engine            : {}", bottle.source);
    println!(
        "Sandboxed         : {} ({})",
        is_sandboxed,
        if is_sandboxed {
            "App Store / Setapp CrossOver"
        } else {
            "direct install"
        }
    );
    println!("drive_c exists    : {}", bottle.exists());
    println!();

    // -----------------------------------------------------------------------
    // drive_X directories present in this bottle
    // -----------------------------------------------------------------------
    let drive_dirs: Vec<String> = {
        let mut v = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&bottle.path) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.starts_with("drive_") && entry.path().is_dir() {
                    v.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
        v.sort();
        v
    };
    println!("=== Drive directories ({}) ===", drive_dirs.len());
    if drive_dirs.is_empty() {
        println!("(none)");
    } else {
        for d in &drive_dirs {
            println!("  {}", d);
        }
    }
    println!();

    // -----------------------------------------------------------------------
    // cxmenu.conf: presence, entry count, and a sample of section headers
    // -----------------------------------------------------------------------
    let cxmenu_path = crate::crossover_cxmenu::find_cxmenu_file(&bottle.path);
    println!("=== cxmenu.conf ===");
    if let Some(ref cxmenu_path) = cxmenu_path {
        match std::fs::read_to_string(cxmenu_path) {
            Ok(content) => {
                let total_lines = content.lines().count();
                let section_headers: Vec<&str> = content
                    .lines()
                    .filter(|l| {
                        let t = l.trim();
                        t.starts_with('[') && t.ends_with(']')
                    })
                    .collect();
                println!("Present    : yes ({})", cxmenu_path.display());
                println!("Total lines: {}", total_lines);
                println!("Sections   : {}", section_headers.len());
                println!("First 5 sections:");
                for header in section_headers.iter().take(5) {
                    // Truncate at a char boundary to avoid panics on multi-byte
                    // UTF-8 (section headers can contain non-ASCII game names).
                    let truncated = if header.chars().count() > 80 {
                        let cut: String = header.chars().take(77).collect();
                        format!("{}...", cut)
                    } else {
                        header.to_string()
                    };
                    println!("  {}", truncated);
                }
            }
            Err(e) => {
                println!("Present    : yes (unreadable: {})", e);
            }
        }
    } else {
        println!("Present    : no");
    }
    println!();

    // -----------------------------------------------------------------------
    // SteamLibrary detection: walk drive_X dirs for Steam paths
    // -----------------------------------------------------------------------
    println!("=== Steam library paths ===");
    let mut steam_library_paths: Vec<String> = Vec::new();

    for drive_dir in &drive_dirs {
        let drive_path = bottle.path.join(drive_dir);

        // Check for a standalone SteamLibrary directory at the drive root
        let standalone = drive_path.join("SteamLibrary");
        if standalone.is_dir() {
            steam_library_paths.push(format!("{} (standalone)", standalone.display()));
        }

        // Check for Steam's libraryfolders.vdf which lists all library roots
        let vdf_path = drive_path
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps")
            .join("libraryfolders.vdf");
        if vdf_path.is_file() {
            steam_library_paths.push(format!(
                "{} (libraryfolders.vdf present)",
                vdf_path.display()
            ));
            // Parse declared library paths from the VDF
            if let Ok(content) = std::fs::read_to_string(&vdf_path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if let Some(rest) = trimmed.strip_prefix("\"path\"") {
                        let rest = rest.trim().trim_matches('"');
                        if !rest.is_empty() {
                            steam_library_paths
                                .push(format!("  declared: {}", rest.replace('\\', "/")));
                        }
                    }
                }
            }
        }

        // Also check the case-variant: Program Files\Steam
        let vdf_path2 = drive_path
            .join("Program Files")
            .join("Steam")
            .join("steamapps")
            .join("libraryfolders.vdf");
        if vdf_path2.is_file() && vdf_path2 != vdf_path {
            steam_library_paths.push(format!(
                "{} (libraryfolders.vdf present)",
                vdf_path2.display()
            ));
        }
    }

    if steam_library_paths.is_empty() {
        println!("(none found)");
    } else {
        for p in &steam_library_paths {
            println!("  {}", p);
        }
    }
    println!();

    // -----------------------------------------------------------------------
    // Per-scan-path .lnk file counts (Start Menu / Desktop)
    // -----------------------------------------------------------------------
    println!("=== .lnk file counts per scan path ===");
    {
        // Mirrors the paths that crossover_shortcuts::scan_bottle_shortcuts walks.
        // We replicate the path logic here so this section is self-contained and
        // doesn't depend on crossover_shortcuts internals.
        let users_dir = bottle.path.join("drive_c").join("users");
        let mut scan_paths: Vec<PathBuf> = Vec::new();

        if users_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&users_dir) {
                for entry in entries.flatten() {
                    let user = entry.path();
                    if !user.is_dir() {
                        continue;
                    }
                    // Windows XP-style Start Menu
                    scan_paths.push(user.join("Start Menu").join("Programs"));
                    // Vista+ style
                    scan_paths.push(
                        user.join("AppData")
                            .join("Roaming")
                            .join("Microsoft")
                            .join("Windows")
                            .join("Start Menu")
                            .join("Programs"),
                    );
                    // Desktop
                    scan_paths.push(user.join("Desktop"));
                }
            }
        }

        // Common (all users) paths
        scan_paths.push(
            bottle
                .path
                .join("drive_c")
                .join("ProgramData")
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );

        let mut any_found = false;
        for scan_path in &scan_paths {
            if !scan_path.exists() {
                continue;
            }
            let lnk_count = walkdir::WalkDir::new(scan_path)
                .follow_links(false)
                .into_iter()
                .flatten()
                .filter(|e| {
                    e.file_type().is_file()
                        && e.path()
                            .extension()
                            .map(|x| x.to_string_lossy().to_lowercase() == "lnk")
                            .unwrap_or(false)
                })
                .count();
            println!("  {} .lnk files  ->  {}", lnk_count, scan_path.display());
            any_found = true;
        }
        if !any_found {
            println!("  (no scan paths exist in this bottle)");
        }
    }
    println!();

    // --- Detected games (Steam appmanifest + plugin-registered) ---
    let games = crate::games::detect_games(&bottle);
    println!("=== Detected games ({}) ===", games.len());
    if games.is_empty() {
        println!("(none)");
    }
    for g in &games {
        println!("- {} [{}]", g.display_name, g.game_id);
        println!("    path: {}", g.game_path.display());
        if let Some(exe) = &g.exe_path {
            println!("    exe:  {}", exe.display());
        }
        println!("    data: {}", g.data_dir.display());

        // Per-game tool detection — useful for confirming ME2 / SKSE etc.
        let tools = crate::mod_tools::detect_tools_for_game(&g.data_dir, &g.game_id);
        let installed: Vec<&crate::mod_tools::ModTool> =
            tools.iter().filter(|t| t.detected_path.is_some()).collect();
        if !installed.is_empty() {
            println!("    tools detected:");
            for t in &installed {
                println!(
                    "      - {} ({}): {}",
                    t.name,
                    t.id,
                    t.detected_path.as_ref().unwrap()
                );
            }
        }
    }
    println!();

    // --- CrossOver shortcuts (the v0.13.x auto-discovery path) ---
    let shortcuts = crate::crossover_shortcuts::scan_bottle_shortcuts(&bottle);
    println!("=== CrossOver shortcuts ({}) ===", shortcuts.len());
    if shortcuts.is_empty() {
        println!("(none — bottle has no .lnk files in any standard Wine location)");
    }
    for s in &shortcuts {
        println!("- {}", s.display_name);
        println!("    lnk:    {}", s.source_lnk_path.display());
        println!("    target: {}", s.windows_target);
        println!("    host:   {}", s.host_target.display());
    }
    println!();

    // --- Unregistered games (shortcut found but no registered game) ---
    let unreg =
        crate::crossover_shortcuts::list_unregistered_games(std::slice::from_ref(&bottle), &games);
    println!(
        "=== Unregistered games (would surface a registration banner) ({}) ===",
        unreg.len()
    );
    if unreg.is_empty() {
        println!("(none)");
    }
    for u in &unreg {
        let hint = u
            .match_hint
            .as_ref()
            .map(|h| format!("{} via {:?}", h.game_id, h.source))
            .unwrap_or_else(|| "no match".into());
        println!(
            "- {} → {} [{}]",
            u.shortcut.display_name,
            u.shortcut.host_target.display(),
            hint
        );
    }
}

/// List detected games as JSON (includes auto-detected Steam games and custom games).
pub fn cli_list_games(db: &Arc<ModDatabase>) {
    let bottles = crate::bottles::detect_bottles();
    let mut all_games: Vec<serde_json::Value> = Vec::new();
    for bottle in &bottles {
        let games = crate::games::detect_games(bottle);
        for game in &games {
            all_games.push(serde_json::json!({
                "id": game.game_id,
                "name": game.display_name,
                "nexus_slug": game.nexus_slug,
                "bottle": bottle.name,
                "path": game.game_path.display().to_string(),
                "executable": game.exe_path.as_ref().map(|p| p.display().to_string()),
            }));
        }
    }
    // Include custom games from DB
    let custom = crate::game_registry::load_custom_games(db);
    for game in &custom {
        if !all_games.iter().any(|g| g["id"] == game.game_id) {
            let bottle_name = game
                .runtime
                .wine()
                .map(|w| w.bottle_name.as_str())
                .unwrap_or("");
            all_games.push(serde_json::json!({
                "id": game.game_id,
                "name": game.display_name,
                "bottle": bottle_name,
                "path": game.game_path.display().to_string(),
                "executable": game.exe_path.as_ref().map(|p| p.display().to_string()),
                "custom": true,
            }));
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&all_games).unwrap_or_default()
    );
}

/// Add a custom game to the database.
pub fn cli_add_game(
    game_id: &str,
    name: &str,
    bottle_name: &str,
    game_path: &str,
    exe_name: Option<&str>,
    mod_dir: Option<&str>,
    nexus_slug: Option<&str>,
    steam_app_id: Option<&str>,
    db: &Arc<ModDatabase>,
) {
    use crate::game_registry::{save_custom_game, CustomGame};

    // Resolve the bottle to get its path
    let bottle = match crate::bottles::find_bottle_by_name(bottle_name) {
        Some(b) => b,
        None => {
            eprintln!("Error: Bottle '{}' not found", bottle_name);
            std::process::exit(1);
        }
    };

    // Resolve game path (could be absolute or relative to bottle's drive_c)
    let full_game_path = if std::path::Path::new(game_path).is_absolute() {
        std::path::PathBuf::from(game_path)
    } else {
        bottle.path.join("drive_c").join(game_path)
    };

    if !full_game_path.is_dir() {
        eprintln!(
            "Error: Game path '{}' does not exist",
            full_game_path.display()
        );
        std::process::exit(1);
    }

    // Find executable
    let exe_path = if let Some(exe) = exe_name {
        let p = full_game_path.join(exe);
        if !p.exists() {
            eprintln!("Warning: Executable '{}' not found at expected path", exe);
        }
        Some(p.display().to_string())
    } else {
        crate::game_registry::find_main_executable_public(&full_game_path)
            .map(|p| p.display().to_string())
    };

    let data_dir = if let Some(md) = mod_dir {
        full_game_path.join(md).display().to_string()
    } else {
        full_game_path.display().to_string()
    };

    let custom = CustomGame {
        game_id: game_id.to_string(),
        display_name: name.to_string(),
        nexus_slug: nexus_slug.unwrap_or(game_id).to_string(),
        game_path: full_game_path.display().to_string(),
        exe_path,
        data_dir,
        bottle_name: bottle.name.clone(),
        bottle_path: bottle.path.display().to_string(),
        steam_app_id: steam_app_id.map(|s| s.to_string()),
    };

    match save_custom_game(db, &custom) {
        Ok(()) => {
            let output = serde_json::json!({
                "ok": true,
                "game_id": custom.game_id,
                "display_name": custom.display_name,
                "game_path": custom.game_path,
                "exe_path": custom.exe_path,
                "data_dir": custom.data_dir,
                "bottle": custom.bottle_name,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

/// Remove a custom game from the database.
pub fn cli_remove_game(game_id: &str, db: &Arc<ModDatabase>) {
    match crate::game_registry::remove_custom_game(db, game_id) {
        Ok(()) => println!("{{\"ok\":true,\"removed\":\"{}\"}}", game_id),
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

/// Database statistics as JSON.
pub fn cli_db_stats(game_id: &str, bottle_name: &str, db: &Arc<ModDatabase>) {
    let (total_mods, enabled_mods) = db.get_mod_counts(game_id, bottle_name).unwrap_or((0, 0));
    let disabled_mods = total_mods.saturating_sub(enabled_mods);
    let deployment_count = db.get_deployment_count(game_id, bottle_name).unwrap_or(0);

    let stats = serde_json::json!({
        "game_id": game_id,
        "bottle_name": bottle_name,
        "total_mods": total_mods,
        "enabled_mods": enabled_mods,
        "disabled_mods": disabled_mods,
        "deployed_files": deployment_count,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&stats).unwrap_or_default()
    );
}

/// SQLite integrity check.
pub fn cli_db_integrity(db: &Arc<ModDatabase>) {
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{{\"ok\":false,\"error\":\"{}\"}}", e);
            std::process::exit(1);
        }
    };

    // PRAGMA integrity_check
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap_or_else(|e| format!("error: {}", e));

    // Count tables
    let table_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    // List tables
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap();
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    // Check schema version
    let schema_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(0);

    let ok = integrity == "ok";
    let result = serde_json::json!({
        "ok": ok,
        "integrity_check": integrity,
        "table_count": table_count,
        "tables": tables,
        "schema_version": schema_version,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );
    if !ok {
        std::process::exit(1);
    }
}

/// List cached Vortex extensions as JSON.
pub fn cli_vortex_list(db: &Arc<ModDatabase>) {
    let summaries = crate::vortex_registry::list_cached(db);
    let json: Vec<serde_json::Value> = summaries
        .iter()
        .map(|s| {
            serde_json::json!({
                "game_id": s.game_id,
                "name": s.name,
                "is_stub": s.is_stub,
                "fetched_at": s.fetched_at,
                "tool_count": s.tool_count,
                "mod_type_count": s.mod_type_count,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&json).unwrap_or_default()
    );
}

/// Check deployment health — verify staged files exist on disk.
pub fn cli_deployment_health(game_id: &str, bottle_name: &str, db: &Arc<ModDatabase>) {
    let mods = match db.list_mods(game_id, bottle_name) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{{\"ok\":false,\"error\":\"{}\"}}", e);
            std::process::exit(1);
        }
    };

    let mut total_mods = 0;
    let mut mods_with_staging = 0;
    let mut staging_exists = 0;
    let mut staging_missing = 0;
    let mut missing_dirs: Vec<String> = Vec::new();

    for m in &mods {
        total_mods += 1;
        if let Some(ref path) = m.staging_path {
            mods_with_staging += 1;
            if std::path::Path::new(path).exists() {
                staging_exists += 1;
            } else {
                staging_missing += 1;
                if missing_dirs.len() < 20 {
                    missing_dirs.push(format!("{}:{}", m.id, m.name));
                }
            }
        }
    }

    let ok = staging_missing == 0;
    let result = serde_json::json!({
        "ok": ok,
        "total_mods": total_mods,
        "mods_with_staging": mods_with_staging,
        "staging_exists": staging_exists,
        "staging_missing": staging_missing,
        "missing_examples": missing_dirs,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );
    if !ok {
        std::process::exit(1);
    }
}

/// List profiles as JSON.
pub fn cli_list_profiles(game_id: &str, bottle_name: &str, db: &Arc<ModDatabase>) {
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{{\"error\":\"{}\"}}", e);
            std::process::exit(1);
        }
    };
    let mut stmt = match conn.prepare(
        "SELECT id, name, is_active FROM profiles WHERE game_id = ?1 AND bottle_name = ?2 ORDER BY name",
    ) {
        Ok(s) => s,
        Err(_) => {
            // profiles table may not exist
            println!("[]");
            return;
        }
    };
    let profiles: Vec<serde_json::Value> = stmt
        .query_map(rusqlite::params![game_id, bottle_name], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "name": row.get::<_, String>(1)?,
                "is_active": row.get::<_, i32>(2)? != 0,
            }))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&profiles).unwrap_or_default()
    );
}

/// Fetch + execute a Vortex extension and report results as JSON.
pub fn cli_vortex_test(game_id: &str) {
    if let Err(e) = vortex_fetcher::validate_game_id(game_id) {
        eprintln!("{{\"ok\":false,\"error\":\"{}\"}}", e);
        std::process::exit(1);
    }

    // Use a tokio runtime for the async fetch
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let source = match rt.block_on(vortex_fetcher::fetch_extension(game_id)) {
        Ok(s) => s,
        Err(e) => {
            let result = serde_json::json!({
                "ok": false,
                "phase": "fetch",
                "error": e,
            });
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            std::process::exit(1);
        }
    };

    println!(
        "[vortex-test] Fetched {}: {} bytes index.js, hash={}",
        game_id,
        source.index_js.len(),
        source.source_hash
    );

    // Execute in QuickJS
    let captured = match vortex_runtime::execute_extension(&source) {
        Ok(c) => c,
        Err(e) => {
            let result = serde_json::json!({
                "ok": false,
                "phase": "execute",
                "error": e,
                "source_bytes": source.index_js.len(),
            });
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            std::process::exit(1);
        }
    };

    // Report what was captured
    let game = captured.game.as_ref();
    let result = serde_json::json!({
        "ok": game.is_some(),
        "game": game.map(|g| serde_json::json!({
            "id": g.id,
            "name": g.name,
            "executable": g.executable,
            "query_mod_path": g.query_mod_path,
            "merge_mods": g.merge_mods,
            "required_files": g.required_files,
            "is_stub": g.is_stub,
            "steam_app_id": g.store_ids.steam_app_id,
            "gog_app_id": g.store_ids.gog_app_id,
            "epic_app_id": g.store_ids.epic_app_id,
            "xbox_id": g.store_ids.xbox_id,
            "steam_dir_name": g.steam_dir_name,
            "tool_count": g.supported_tools.len(),
            "tools": g.supported_tools.iter().map(|t| serde_json::json!({
                "id": t.id,
                "name": t.name,
                "executable": t.executable,
            })).collect::<Vec<_>>(),
        })),
        "mod_types": captured.mod_types.iter().map(|mt| serde_json::json!({
            "id": mt.id,
            "priority": mt.priority,
            "target_path": mt.target_path,
        })).collect::<Vec<serde_json::Value>>(),
        "installers": captured.installers.iter().map(|i| serde_json::json!({
            "id": i.id,
            "priority": i.priority,
        })).collect::<Vec<serde_json::Value>>(),
    });

    println!("{}", serde_json::to_string_pretty(&result).unwrap());
}

// --- CLI shader scan test ---

/// Scan installed mods for Community Shaders dependencies and print results as JSON.
///
/// Usage:  corkscrew --test-shader-scan <game_id> <bottle_name>
/// Example: corkscrew --test-shader-scan skyrimse Steam
pub fn cli_test_shader_scan(game_id: &str, bottle_name: &str, db: &Arc<ModDatabase>) {
    println!(
        "[corkscrew] Scanning for Community Shaders mods: game={} bottle={}",
        game_id, bottle_name
    );

    let (_, game, _) = match resolve_game(game_id, bottle_name) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[corkscrew] ERROR resolving game: {}", e);
            std::process::exit(1);
        }
    };
    let game_path = PathBuf::from(&game.game_path);

    match shader_conversion::scan_for_cs_mods(db, game_id, bottle_name, &game_path) {
        Ok(result) => {
            println!("[corkscrew] Scan complete:");
            println!("  Total CS mods detected:  {}", result.total_cs_mods);
            println!("  Swappable (ENB variant): {}", result.swappable_count);
            println!("  FOMOD re-run needed:     {}", result.fomod_rerun_count);
            println!("  Disable-only:            {}", result.disable_only_count);
            println!(
                "  ENB already installed:   {}",
                result.enb_already_installed
            );
            println!();
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
        }
        Err(e) => {
            eprintln!("[corkscrew] ERROR scanning for CS mods: {}", e);
            std::process::exit(1);
        }
    }
}

// --- CLI headless launch ---

/// Run the full pre-launch pipeline and spawn the game without opening the UI.
///
/// Usage:  corkscrew --launch <game_id> <bottle_name> [--skse]
/// Example: corkscrew --launch skyrimse Steam --skse
pub fn cli_launch(game_id: &str, bottle_name: &str, use_skse: bool, db: &Arc<ModDatabase>) {
    println!(
        "[corkscrew] --launch mode: game={} bottle={} skse={}",
        game_id, bottle_name, use_skse
    );

    let (bottle, game, _) = match resolve_game(game_id, bottle_name) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[corkscrew] ERROR: {}", e);
            std::process::exit(1);
        }
    };
    let game_path = PathBuf::from(&game.game_path);

    let exe_name = if use_skse && game_id == "skyrimse" {
        "skse64_loader.exe".to_string()
    } else {
        games::with_plugin(game_id, |plugin| {
            plugin
                .executables()
                .first()
                .map(|s| s.to_string())
                .unwrap_or_default()
        })
        .unwrap_or_default()
    };

    if exe_name.is_empty() {
        eprintln!(
            "[corkscrew] ERROR: No executable configured for game '{}'",
            game_id
        );
        std::process::exit(1);
    }

    let exe_path = match launcher::find_executable(&game_path, &exe_name) {
        Some(p) => p,
        None => {
            eprintln!(
                "[corkscrew] ERROR: {} not found in {}",
                exe_name,
                game_path.display()
            );
            std::process::exit(1);
        }
    };

    let fixes_disabled = config::get_config_value("disable_game_fixes")
        .unwrap_or(None)
        .map(|v| v == "true")
        .unwrap_or(false);

    if game_id == "skyrimse" && !fixes_disabled {
        match display_fix::auto_fix_display(&bottle) {
            Ok(r) => {
                if r.fixed {
                    println!(
                        "[corkscrew] Display fix applied: {}x{} fullscreen",
                        r.applied.width, r.applied.height
                    );
                }
            }
            Err(e) => eprintln!("[corkscrew] Warning: display fix failed: {}", e),
        }
    }

    let _ = crate::sync_plugins_for_game(&game, &bottle);

    if game_id == "skyrimse" {
        let data_dir = PathBuf::from(&game.data_dir);

        let fixes =
            skse::fix_skse_plugin_conflicts(db, game_id, bottle_name, &data_dir, &game_path);
        if fixes > 0 {
            println!("[corkscrew] Fixed {} SKSE plugin DLL(s)", fixes);
        }

        let use_wine_ef = config::get_config()
            .map(|c| c.use_wine_engine_fixes)
            .unwrap_or(false);

        if use_wine_ef {
            let ef = skse::fix_engine_fixes_for_wine(&data_dir, db, game_id, bottle_name);
            if ef > 0 {
                println!("[corkscrew] Patched {} EngineFixes TOML(s) for Wine", ef);
            }
        } else {
            println!("[corkscrew] Skipping Wine EngineFixes (not enabled — enable in Settings)");
        }

        let wine_disabled =
            skse::disable_wine_incompatible_plugins(&data_dir, db, game_id, bottle_name);
        for (name, _reason) in &wine_disabled {
            println!("[corkscrew] Disabled Wine-incompatible plugin: {}", name);
        }

        if use_wine_ef {
            match skse::install_engine_fixes_wine_blocking(&data_dir) {
                Ok(true) => println!("[corkscrew] Deployed SSE Engine Fixes for Wine"),
                Ok(false) => println!("[corkscrew] SSE Engine Fixes for Wine already up to date"),
                Err(e) => eprintln!("[corkscrew] Warning: Engine Fixes deploy failed: {}", e),
            }
        }
    }

    println!("[corkscrew] Launching {} ...", exe_path.display());
    match launcher::launch_game(&bottle, &exe_path, Some(&game_path), Some(game_id), None) {
        Ok(r) => {
            println!("[corkscrew] Launched OK (pid={:?})", r.pid);
            if let Some(w) = r.warning {
                eprintln!("[corkscrew] Warning: {}", w);
            }
        }
        Err(e) => {
            eprintln!("[corkscrew] ERROR: Launch failed: {}", e);
            std::process::exit(1);
        }
    }
}

// --- Startup Cleanup ---

/// Mark orphaned Wabbajack installs (left in active state from a crash) as failed
/// and clean up their extraction temp directories.
pub fn cleanup_orphaned_wj_installs(db: &database::ModDatabase) {
    match db.get_stale_wj_installs() {
        Ok(stale) => {
            for (id, install_dir, status) in &stale {
                log::warn!(
                    "Found orphaned WJ install {} (status={}) — marking as failed",
                    id,
                    status
                );
                let _ = db.update_wj_install_status(
                    *id,
                    "failed",
                    Some("Interrupted by application exit"),
                );
                // Clean up extraction temp dir if it still exists
                let temp_dir = std::path::Path::new(install_dir).join(".wj_extraction_temp");
                if temp_dir.exists() {
                    log::info!("Removing orphaned extraction temp dir: {:?}", temp_dir);
                    let _ = std::fs::remove_dir_all(&temp_dir);
                }
            }
            if !stale.is_empty() {
                log::info!("Cleaned up {} orphaned WJ install(s)", stale.len());
            }
        }
        Err(e) => log::warn!("Failed to query stale WJ installs: {}", e),
    }
}

/// Remove any leftover `corkscrew_extract_*` directories from the system temp dir.
/// These are created by the collection installer and should be cleaned up on
/// completion, but may be orphaned if the app crashes during extraction.
pub fn cleanup_orphaned_temp_dirs() {
    let temp = std::env::temp_dir();
    match std::fs::read_dir(&temp) {
        Ok(entries) => {
            let mut cleaned = 0u32;
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with("corkscrew_extract_") && entry.path().is_dir() {
                        log::info!("Removing orphaned temp dir: {:?}", entry.path());
                        let _ = std::fs::remove_dir_all(entry.path());
                        cleaned += 1;
                    }
                }
            }
            if cleaned > 0 {
                log::info!("Cleaned up {} orphaned temp dir(s)", cleaned);
            }
        }
        Err(e) => log::warn!("Failed to scan temp dir for orphans: {}", e),
    }
}
