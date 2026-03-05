<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import {
    getWabbajackModlists,
    getConfig,
    fetchUrlText,
    parseWabbajackFile,
    downloadWabbajackFile,
    detectWabbajackTools,
    installWabbajackModlist,
    cancelWabbajackInstall,
    cleanupWabbajackInstall,
    closeBrowserWebview,
    getPendingWabbajackInstalls,
  } from "$lib/api";
  import { showError, showSuccess, selectedGame } from "$lib/stores";
  import type { ModlistSummary, ParsedModlist, RequiredTool, WabbajackInstallStatus, WjArchiveStatus, WjInstallProgressEvent } from "$lib/types";
  import { SpeedTracker } from "$lib/speedTracker";
  import { marked } from "marked";
  import DOMPurify from "dompurify";
  import { bbcodeToHtml } from "$lib/bbcode";
  import CompatibilityPanel from "$lib/components/CompatibilityPanel.svelte";
  import RequiredToolsPrompt from "$lib/components/RequiredToolsPrompt.svelte";
  import WabbajackLogo from "$lib/components/WabbajackLogo.svelte";
  import WebViewToggle from "$lib/components/WebViewToggle.svelte";

  let modlists = $state<ModlistSummary[]>([]);
  let filtered = $state<ModlistSummary[]>([]);
  let loading = $state(true);
  let searchQuery = $state("");
  let gameFilter = $state("all");
  let nsfwFilter = $state<"hide" | "show" | "only">("hide");
  let sortField = $state<"title" | "author" | "download_size" | "install_size">("title");
  let sortDirection = $state<"asc" | "desc">("asc");

  // Advanced filter state
  let maxInstallSize = $state<number | null>(null);
  let tagFilter = $state<string[]>([]);
  let showAdvancedFilters = $state(false);

  // WebView toggle state
  let webviewToggle: WebViewToggle | null = $state(null);
  let viewMode = $state<"app" | "website">("app");

  // Detail view state
  let selectedModlist = $state<ModlistSummary | null>(null);
  let readmeContent = $state("");
  let loadingDetail = $state(false);

  // Parsed file state (after loading .wabbajack)
  let parsedModlist = $state<ParsedModlist | null>(null);
  let wabbajackFilePath = $state<string | null>(null);
  let parsingFile = $state(false);

  // Download state
  let downloading = $state(false);
  let downloadError = $state<string | null>(null);

  // Tool detection state
  let pendingTools = $state<RequiredTool[]>([]);
  let showToolsPrompt = $state(false);

  // Install state
  let installing = $state(false);
  let installStep = $state("");
  let wjInstallId = $state<number | null>(null);
  let wjPhase = $state("");
  let wjCurrent = $state(0);
  let wjTotal = $state(0);
  let wjCurrentFile = $state("");
  let wjUnlisten: UnlistenFn | null = null;

  // Speed tracking
  let wjSpeedTracker = new SpeedTracker();
  let wjStartTime = $state(0);
  let wjElapsed = $state("");
  let wjSpeed = $state(0);
  let wjEta = $state("");
  let wjTotalBytes = $state(0);
  let wjBytesCompleted = $state(0);
  let wjOverallProgress = $state(0);
  let wjArchives = $state<WjArchiveStatus[]>([]);
  let wjElapsedTimer: ReturnType<typeof setInterval> | null = null;

  // Resume banner state
  let pendingWjInstall = $state<WabbajackInstallStatus | null>(null);
  let resumingWj = $state(false);

  // Derived unique games from the modlists
  const gameOptions = $derived.by(() => {
    const games = new Set(modlists.map((m) => m.game));
    return Array.from(games).sort();
  });

  // Derived available tags from all modlists
  const availableTags = $derived.by(() => {
    const tags = new Set<string>();
    modlists.forEach((m) => m.tags?.forEach((t: string) => tags.add(t)));
    return Array.from(tags).sort();
  });

  // Count of active advanced filters
  const activeFilterCount = $derived(
    (maxInstallSize !== null ? 1 : 0) + tagFilter.length
  );

  $effect(() => {
    let result = modlists;

    // Filter by NSFW
    if (nsfwFilter === "hide") {
      result = result.filter((m) => !m.nsfw);
    } else if (nsfwFilter === "only") {
      result = result.filter((m) => m.nsfw);
    }

    // Filter by game
    if (gameFilter !== "all") {
      result = result.filter((m) => m.game === gameFilter);
    }

    // Filter by search
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      result = result.filter(
        (m) =>
          m.title.toLowerCase().includes(q) ||
          m.author.toLowerCase().includes(q) ||
          m.description.toLowerCase().includes(q) ||
          m.tags.some((t) => t.toLowerCase().includes(q))
      );
    }

    // Filter by max install size
    if (maxInstallSize !== null) {
      const limit = maxInstallSize;
      result = result.filter((m) => m.install_size <= limit);
    }

    // Filter by tags
    if (tagFilter.length > 0) {
      result = result.filter((m) =>
        tagFilter.every((t) => m.tags?.includes(t))
      );
    }

    // Sort
    result = [...result].sort((a, b) => {
      let cmp = 0;
      if (sortField === "title") cmp = a.title.localeCompare(b.title);
      else if (sortField === "author") cmp = a.author.localeCompare(b.author);
      else cmp = (a[sortField] as number) - (b[sortField] as number);
      return sortDirection === "asc" ? cmp : -cmp;
    });

    filtered = result;
  });

  function cycleNsfwFilter(current: "hide" | "show" | "only"): "hide" | "show" | "only" {
    if (current === "hide") return "show";
    if (current === "show") return "only";
    return "hide";
  }

  function nsfwLabel(state: "hide" | "show" | "only"): string {
    if (state === "hide") return "NSFW Off";
    if (state === "show") return "NSFW On";
    return "NSFW Only";
  }

  function nsfwIcon(state: "hide" | "show" | "only"): string {
    if (state === "hide") return "";
    if (state === "show") return "\u2713";
    return "\u2500";
  }

  onDestroy(() => {
    closeBrowserWebview().catch(() => {});
    cleanupWjListener();
  });

  onMount(async () => {
    try {
      modlists = await getWabbajackModlists();
    } catch (e: unknown) {
      showError(`Failed to load modlists: ${e}`);
    } finally {
      loading = false;
    }

    // Check for interrupted Wabbajack installs
    try {
      const pending = await getPendingWabbajackInstalls();
      const resumable = pending.find(
        (p) => p.status !== "completed" && p.status !== "cancelled"
      );
      if (resumable) {
        pendingWjInstall = resumable;
      }
    } catch {
      // Silently ignore — resume banner is non-critical
    }
  });

  function formatSize(bytes: number): string {
    if (bytes === 0) return "N/A";
    const gb = bytes / (1024 * 1024 * 1024);
    if (gb >= 1) return `${gb.toFixed(1)} GB`;
    const mb = bytes / (1024 * 1024);
    return `${mb.toFixed(0)} MB`;
  }

  function gameDomainDisplay(domain: string): string {
    const map: Record<string, string> = {
      skyrim: "Skyrim LE",
      skyrimspecialedition: "Skyrim SE",
      skyrimvr: "Skyrim VR",
      fallout4: "Fallout 4",
      fallout4vr: "Fallout 4 VR",
      falloutnewvegas: "Fallout NV",
      fallout3: "Fallout 3",
      oblivion: "Oblivion",
      morrowind: "Morrowind",
      enderal: "Enderal",
      enderalspecialedition: "Enderal SE",
      cyberpunk2077: "Cyberpunk 2077",
      stardewvalley: "Stardew Valley",
      witcher3: "Witcher 3",
      thewitcher3: "Witcher 3",
      starfield: "Starfield",
      baldursgate3: "BG3",
    };
    return map[domain] || domain;
  }

  async function viewModlistDetail(modlist: ModlistSummary) {
    selectedModlist = modlist;
    readmeContent = "";
    loadingDetail = true;

    if (modlist.readme_url) {
      try {
        const raw = await fetchUrlText(modlist.readme_url);
        // If the content looks like a full HTML page, use it directly (sanitized).
        // Otherwise, treat it as markdown and render it.
        const trimmed = raw.trimStart();
        let htmlResult: string;
        if (trimmed.startsWith("<!DOCTYPE") || trimmed.startsWith("<html")) {
          htmlResult = raw;
        } else {
          htmlResult = await marked.parse(raw);
        }
        // Rewrite relative image/link URLs to absolute based on the readme source
        htmlResult = rewriteRelativeUrls(htmlResult, modlist.readme_url);
        readmeContent = DOMPurify.sanitize(htmlResult);
      } catch (e: unknown) {
        const errMsg = String(e);
        const is404 = errMsg.includes("404");
        readmeContent = DOMPurify.sanitize(
          `<div style="color: var(--text-tertiary); text-align: center; padding: 24px 0;">` +
          `<p style="margin-bottom: 8px;">${is404 ? "Readme page is no longer available." : "Could not load readme."}</p>` +
          `<p style="font-size: 12px; opacity: 0.7;">${is404 ? "The author's readme link may have moved or been removed." : errMsg}</p>` +
          `</div>`,
        );
      }
    }

    loadingDetail = false;
  }

  function backToBrowse() {
    selectedModlist = null;
    readmeContent = "";
    parsedModlist = null;
    wabbajackFilePath = null;
  }

  /** Open a file picker to select a local .wabbajack file. */
  async function openLocalFile() {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: "Wabbajack Modlist", extensions: ["wabbajack"] }],
      });
      if (!selected) return;

      const filePath = typeof selected === "string" ? selected : selected;
      await parseAndShowFile(filePath);
    } catch (e: unknown) {
      showError(`Failed to open file: ${e}`);
    }
  }

  /** Parse a .wabbajack file and show the summary view. */
  async function parseAndShowFile(filePath: string) {
    parsingFile = true;
    parsedModlist = null;
    wabbajackFilePath = filePath;
    // Clear gallery detail view
    selectedModlist = null;
    readmeContent = "";

    try {
      parsedModlist = await parseWabbajackFile(filePath);
    } catch (e: unknown) {
      showError(`Failed to parse .wabbajack file: ${e}`);
      parsedModlist = null;
      wabbajackFilePath = null;
    } finally {
      parsingFile = false;
    }
  }

  /** Group archives by source type for display. */
  function archiveBreakdown(modlist: ParsedModlist): { source: string; count: number; size: number }[] {
    const map = new Map<string, { count: number; size: number }>();
    for (const archive of modlist.archives) {
      const entry = map.get(archive.source_type) ?? { count: 0, size: 0 };
      entry.count += 1;
      entry.size += archive.size;
      map.set(archive.source_type, entry);
    }
    return Array.from(map.entries())
      .map(([source, data]) => ({ source, ...data }))
      .sort((a, b) => b.count - a.count);
  }

  /** Check for required tools and begin install. */
  async function handleBeginInstall() {
    const game = $selectedGame;
    if (!game || !wabbajackFilePath) return;

    try {
      const tools = await detectWabbajackTools(wabbajackFilePath, game.game_id, game.bottle_name);
      const uninstalled = tools.filter((t) => !t.is_detected);
      if (uninstalled.length > 0) {
        pendingTools = tools;
        showToolsPrompt = true;
        return;
      }
      proceedWithInstall();
    } catch (e: unknown) {
      showError(`Tool detection failed: ${e}`);
      proceedWithInstall();
    }
  }

  async function proceedWithInstall() {
    const game = $selectedGame;
    if (!game || !wabbajackFilePath) return;

    showToolsPrompt = false;
    pendingTools = [];
    installing = true;
    installStep = "Preparing installation...";
    wjPhase = "";
    wjCurrent = 0;
    wjTotal = 0;
    wjCurrentFile = "";
    wjStartTime = Date.now();
    wjElapsed = "0s";
    wjArchives = [];
    wjSpeed = 0;
    wjEta = "";
    wjTotalBytes = 0;
    wjBytesCompleted = 0;
    wjOverallProgress = 0;
    wjSpeedTracker.reset();
    wjElapsedTimer = setInterval(() => {
      wjElapsed = SpeedTracker.formatElapsed(wjStartTime);
      // Compute overall progress as weighted average
      // Download=30%, Extraction=20%, Directives=30%, Deploy=20%
      if (wjPhase === "downloading" && wjTotal > 0) wjOverallProgress = Math.round((wjCurrent / wjTotal) * 30);
      else if (wjPhase === "extracting" && wjTotal > 0) wjOverallProgress = 30 + Math.round((wjCurrent / wjTotal) * 20);
      else if (wjPhase === "directives" && wjTotal > 0) wjOverallProgress = 50 + Math.round((wjCurrent / wjTotal) * 30);
      else if (wjPhase === "deploying" && wjTotal > 0) wjOverallProgress = 80 + Math.round((wjCurrent / wjTotal) * 20);
      else if (wjPhase === "done") wjOverallProgress = 100;
    }, 1000);

    // Determine paths
    let downloadDir: string;
    try {
      const config = await getConfig();
      downloadDir = config.download_dir || `${game.bottle_path}/drive_c/corkscrew_downloads`;
    } catch {
      downloadDir = `${game.bottle_path}/drive_c/corkscrew_downloads`;
    }
    const installDir = game.data_dir;

    // Subscribe to progress events
    try {
      wjUnlisten = await listen<WjInstallProgressEvent>("wj-install-progress", (event) => {
        const p = event.payload;
        switch (p.type) {
          case "PreFlightStarted":
            wjPhase = "preflight";
            installStep = "Running preflight checks...";
            break;
          case "PreFlightCompleted":
            if (!p.report.can_proceed) {
              const issues = p.report.issues.map((i) => i.message).join("; ");
              installStep = `Preflight issues: ${issues}`;
            } else {
              installStep = `Preflight OK — ${p.report.total_archives} archives, ${p.report.cached_archives} cached`;
            }
            break;
          case "DownloadPhaseStarted":
            wjPhase = "downloading";
            wjTotal = p.total;
            wjCurrent = 0;
            wjSpeedTracker.reset();
            wjBytesCompleted = 0;
            installStep = `Downloading archives (0/${p.total})...`;
            break;
          case "DownloadStarted":
            wjCurrent = p.index + 1;
            wjCurrentFile = p.name;
            installStep = `Downloading ${p.name} (${wjCurrent}/${wjTotal})...`;
            break;
          case "DownloadProgress":
            if (p.total_bytes > 0) {
              wjBytesCompleted = p.bytes;
              wjTotalBytes = p.total_bytes;
              wjSpeed = wjSpeedTracker.update(p.bytes);
              wjEta = SpeedTracker.formatEta(p.total_bytes - p.bytes, wjSpeed);
              const dlPct = Math.round((p.bytes / p.total_bytes) * 100);
              const dlSpeedStr = SpeedTracker.formatSpeed(wjSpeed);
              installStep = `Downloading ${p.name} — ${dlPct}%${dlSpeedStr ? ` — ${dlSpeedStr}` : ""} (${wjCurrent}/${wjTotal})`;
            }
            break;
          case "DownloadCompleted":
            installStep = `Downloaded ${p.name} (${wjCurrent}/${wjTotal})`;
            break;
          case "DownloadFailed":
            installStep = `Failed: ${p.name} — ${p.error}`;
            break;
          case "DownloadSkipped":
            installStep = `Skipped ${p.name}: ${p.reason}`;
            break;
          case "ExtractionStarted":
            wjPhase = "extracting";
            wjTotal = p.total;
            wjCurrent = 0;
            wjTotalBytes = p.total_bytes ?? 0;
            wjBytesCompleted = 0;
            wjSpeedTracker.reset();
            installStep = `Extracting archives (0/${p.total})...`;
            break;
          case "ExtractionArchiveStarted":
            wjArchives = [...wjArchives.filter(a => a.index !== p.index), { name: p.name, index: p.index, size: p.size, status: "extracting" as const }].sort((a, b) => a.index - b.index);
            wjCurrentFile = p.name;
            installStep = `Extracting ${p.name} (${wjCurrent + 1}/${wjTotal})...`;
            break;
          case "ExtractionArchiveCompleted":
            wjArchives = wjArchives.map(a => a.index === p.index ? { ...a, status: "extracted" as const } : a);
            break;
          case "ExtractionArchiveFailed":
            wjArchives = wjArchives.map(a => a.name === p.name ? { ...a, status: "failed" as const, error: p.error } : a);
            break;
          case "ExtractionProgress":
            wjCurrent = p.index + 1;
            wjCurrentFile = p.name;
            if (p.total_bytes > 0) {
              wjBytesCompleted = p.bytes_completed ?? 0;
              wjTotalBytes = p.total_bytes;
              wjSpeed = wjSpeedTracker.update(wjBytesCompleted);
              const extSpeedStr = SpeedTracker.formatSpeed(wjSpeed);
              installStep = `Extracting ${p.name}${extSpeedStr ? ` — ${extSpeedStr}` : ""} (${wjCurrent}/${wjTotal})`;
            } else {
              installStep = `Extracting ${p.name} (${wjCurrent}/${wjTotal})...`;
            }
            break;
          case "DirectivePhaseStarted":
            wjPhase = "directives";
            wjTotal = p.total;
            wjCurrent = 0;
            wjTotalBytes = p.total_bytes ?? 0;
            wjBytesCompleted = 0;
            wjSpeedTracker.reset();
            installStep = `Processing directives (0/${p.total.toLocaleString()})...`;
            break;
          case "DirectiveProgress":
            wjCurrent = p.current;
            wjCurrentFile = p.current_file ?? "";
            if (p.total_bytes > 0 && p.bytes_processed > 0) {
              wjBytesCompleted = p.bytes_processed;
              wjTotalBytes = p.total_bytes;
              wjSpeed = wjSpeedTracker.update(p.bytes_processed);
              const dirSpeedStr = SpeedTracker.formatSpeed(wjSpeed);
              installStep = `Processing ${p.directive_type} — ${SpeedTracker.formatBytes(p.bytes_processed)} / ${SpeedTracker.formatBytes(p.total_bytes)}${dirSpeedStr ? ` — ${dirSpeedStr}` : ""} (${p.current.toLocaleString()}/${p.total.toLocaleString()})`;
            } else {
              installStep = `Processing ${p.directive_type} (${p.current.toLocaleString()}/${p.total.toLocaleString()})...`;
            }
            break;
          case "DeployStarted":
            wjPhase = "deploying";
            wjTotal = p.total;
            wjCurrent = 0;
            wjTotalBytes = p.total_bytes ?? 0;
            wjBytesCompleted = 0;
            wjCurrentFile = p.modlist_name ?? "";
            wjSpeedTracker.reset();
            installStep = `Deploying ${p.modlist_name ? `"${p.modlist_name}" ` : ""}files (0/${p.total.toLocaleString()})...`;
            break;
          case "DeployProgress":
            wjCurrent = p.current;
            if (p.total_bytes > 0 && p.bytes_deployed > 0) {
              wjBytesCompleted = p.bytes_deployed;
              wjSpeed = wjSpeedTracker.update(p.bytes_deployed);
              const depSpeedStr = SpeedTracker.formatSpeed(wjSpeed);
              installStep = `Deploying files — ${SpeedTracker.formatBytes(p.bytes_deployed)} / ${SpeedTracker.formatBytes(p.total_bytes)}${depSpeedStr ? ` — ${depSpeedStr}` : ""} (${p.current.toLocaleString()}/${p.total.toLocaleString()})`;
            } else {
              installStep = `Deploying files (${p.current.toLocaleString()}/${p.total.toLocaleString()})...`;
            }
            break;
          case "InstallCompleted": {
            const r = p.result;
            wjPhase = "done";
            installing = false;
            installStep = `Installed ${r.files_deployed.toLocaleString()} files in ${r.elapsed_secs.toFixed(0)}s`;
            showSuccess(`Modlist installed successfully — ${r.files_deployed.toLocaleString()} files deployed`);
            cleanupWjListener();
            break;
          }
          case "InstallFailed":
            wjPhase = "error";
            installing = false;
            installStep = `Installation failed: ${p.error}`;
            showError(`Modlist installation failed: ${p.error}`);
            cleanupWjListener();
            break;
          case "InstallCancelled":
            wjPhase = "";
            installing = false;
            installStep = "Installation cancelled.";
            cleanupWjListener();
            break;
          case "UserActionRequired":
            installStep = `Manual download required: ${p.archive_name}`;
            openUrl(p.url);
            break;
        }
      });
    } catch (e) {
      showError(`Failed to subscribe to install events: ${e}`);
      installing = false;
      return;
    }

    // Start the install
    try {
      wjInstallId = await installWabbajackModlist(
        wabbajackFilePath, game.game_id, game.bottle_name,
        installDir, downloadDir,
      );
    } catch (e: unknown) {
      installing = false;
      installStep = "";
      showError(`Failed to start installation: ${e}`);
      cleanupWjListener();
    }
  }

  function cleanupWjListener() {
    if (wjUnlisten) {
      wjUnlisten();
      wjUnlisten = null;
    }
    if (wjElapsedTimer) {
      clearInterval(wjElapsedTimer);
      wjElapsedTimer = null;
    }
    if (wjInstallId !== null) {
      cleanupWabbajackInstall(wjInstallId).catch(() => {});
    }
    wjInstallId = null;
  }

  async function handleCancelInstall() {
    if (wjInstallId !== null) {
      try {
        await cancelWabbajackInstall(wjInstallId);
      } catch (e) {
        showError(`Failed to cancel: ${e}`);
      }
    }
  }

  function sourceLabel(source: string): string {
    const labels: Record<string, string> = {
      Nexus: "Nexus Mods",
      HTTP: "Direct Download",
      GoogleDrive: "Google Drive",
      Mega: "Mega.nz",
      MediaFire: "MediaFire",
      ModDB: "ModDB",
      WabbajackCDN: "Wabbajack CDN",
      GameFile: "Game Files",
      Manual: "Manual Download",
    };
    return labels[source] ?? source;
  }

  function sourceColor(source: string): string {
    const colors: Record<string, string> = {
      Nexus: "var(--system-accent)",
      HTTP: "#22c55e",
      WabbajackCDN: "#a78bfa",
      GoogleDrive: "#f59e0b",
      Mega: "#ef4444",
      GameFile: "var(--text-tertiary)",
      Manual: "#f97316",
    };
    return colors[source] ?? "var(--text-secondary)";
  }

  async function handleDownloadWabbajack(modlist: ModlistSummary) {
    if (!modlist.download_url || downloading) return;
    downloading = true;
    downloadError = null;
    try {
      // Sanitize filename from title
      const safeName = modlist.title.replace(/[^a-zA-Z0-9_\- ]/g, "").trim();
      const filename = `${safeName}.wabbajack`;
      const filePath = await downloadWabbajackFile(modlist.download_url, filename);
      showSuccess(`Downloaded "${modlist.title}" — parsing...`);

      // Auto-parse the downloaded file
      parsedModlist = await parseWabbajackFile(filePath);
      wabbajackFilePath = filePath;
      selectedModlist = null;
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      downloadError = msg;
      showError(`Download failed: ${msg}`);
    } finally {
      downloading = false;
    }
  }

  /** Validate that a URL is a safe HTTP(S) URL before opening in browser. */
  function safeOpenUrl(url: string | null | undefined) {
    if (!url) return;
    try {
      const parsed = new URL(url);
      if (parsed.protocol === "http:" || parsed.protocol === "https:") {
        openUrl(url);
      }
    } catch {
      // ignore invalid URLs
    }
  }

  /** Rewrite relative image src and link href URLs to absolute based on a base URL. */
  function rewriteRelativeUrls(html: string, baseUrl: string): string {
    try {
      const base = new URL(baseUrl);
      // For GitHub wiki URLs, derive the raw content base
      // e.g. https://raw.githubusercontent.com/.../wiki/Home.md → base is the directory
      const baseDir = base.href.substring(0, base.href.lastIndexOf("/") + 1);

      // Rewrite src="..." and href="..." that are relative
      return html.replace(/(src|href)="([^"]+)"/g, (_match, attr, url) => {
        if (url.startsWith("http://") || url.startsWith("https://") || url.startsWith("data:") || url.startsWith("#") || url.startsWith("mailto:")) {
          return `${attr}="${url}"`;
        }
        try {
          const absolute = new URL(url, baseDir).href;
          return `${attr}="${absolute}"`;
        } catch {
          return `${attr}="${url}"`;
        }
      });
    } catch {
      return html;
    }
  }

  /** Intercept clicks on links inside rendered markdown/HTML and open them externally. */
  function handleRenderedLinkClick(e: MouseEvent) {
    const target = (e.target as HTMLElement)?.closest("a");
    if (!target) return;
    const href = target.getAttribute("href");
    if (href) {
      e.preventDefault();
      e.stopPropagation();
      safeOpenUrl(href);
    }
  }

  function handleDismissWjInstall() {
    pendingWjInstall = null;
  }
</script>

<div class="modlists-page">
  {#if pendingWjInstall}
    <div class="resume-banner">
      <div class="resume-info">
        <span class="resume-icon">&#9888;</span>
        <div class="resume-text">
          <span class="resume-title">Interrupted Installation</span>
          <span class="resume-detail">
            "{pendingWjInstall.modlist_name}" &mdash; {pendingWjInstall.completed_archives}/{pendingWjInstall.total_archives} archives downloaded
          </span>
        </div>
      </div>
      <div class="resume-actions">
        <button class="btn-ghost" onclick={handleDismissWjInstall}>Dismiss</button>
      </div>
    </div>
  {/if}

  {#if selectedModlist}
    <!-- Detail View -->
    <div class="detail-view">
      <div class="detail-header">
        <button class="btn btn-ghost" onclick={backToBrowse}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M19 12H5" />
            <polyline points="12 19 5 12 12 5" />
          </svg>
          Back to Browse
        </button>
      </div>

      <div class="detail-content">
        <!-- Title Section -->
        <div class="detail-title-section">
          <div class="detail-title-row">
            <h2 class="detail-name">{selectedModlist.title}</h2>
            <span class="game-badge">{gameDomainDisplay(selectedModlist.game)}</span>
            {#if selectedModlist.nsfw}
              <span class="nsfw-badge">NSFW</span>
            {/if}
          </div>
          <p class="detail-author">by {selectedModlist.author}</p>
          <span class="detail-version">v{selectedModlist.version}</span>
        </div>

        <!-- Stats Bar -->
        <div class="detail-stats-bar">
          <div class="detail-stat">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
              <polyline points="7 10 12 15 17 10" />
              <line x1="12" y1="15" x2="12" y2="3" />
            </svg>
            <span class="detail-stat-value">{formatSize(selectedModlist.download_size)}</span>
            <span class="detail-stat-label">Download</span>
          </div>
          <div class="detail-stat">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
            </svg>
            <span class="detail-stat-value">{formatSize(selectedModlist.install_size)}</span>
            <span class="detail-stat-label">Installed</span>
          </div>
          <div class="detail-stat">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
            </svg>
            <span class="detail-stat-value">{selectedModlist.archive_count.toLocaleString()}</span>
            <span class="detail-stat-label">Archives</span>
          </div>
          <div class="detail-stat">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" />
              <polyline points="13 2 13 9 20 9" />
            </svg>
            <span class="detail-stat-value">{selectedModlist.file_count.toLocaleString()}</span>
            <span class="detail-stat-label">Files</span>
          </div>
        </div>

        <!-- Description -->
        {#if selectedModlist.description}
          <div class="detail-section">
            <h3 class="detail-section-title">Description</h3>
            <div class="detail-description rendered-markdown">
              {@html DOMPurify.sanitize(bbcodeToHtml(selectedModlist.description))}
            </div>
          </div>
        {/if}

        <!-- Tags -->
        {#if selectedModlist.tags.length > 0}
          <div class="detail-section">
            <h3 class="detail-section-title">Tags</h3>
            <div class="detail-tags">
              {#each selectedModlist.tags as tag}
                <span class="tag">{tag}</span>
              {/each}
            </div>
          </div>
        {/if}

        <!-- Readme -->
        <div class="detail-section">
          <h3 class="detail-section-title">Readme</h3>
          {#if loadingDetail}
            <div class="readme-loading">
              <div class="spinner-sm-ring"></div>
              <span>Loading readme...</span>
            </div>
          {:else if readmeContent}
            <div class="rendered-markdown" onclick={handleRenderedLinkClick}>
              {@html readmeContent}
            </div>
          {:else}
            <p class="detail-no-readme">No readme available for this modlist.</p>
          {/if}
        </div>

        <!-- Compatibility Check (Skyrim SE only) -->
        {#if selectedModlist.game === "skyrimspecialedition" && $selectedGame}
          <div class="detail-section">
            <CompatibilityPanel gameId={$selectedGame.game_id} bottleName={$selectedGame.bottle_name} />
          </div>
        {/if}

        <!-- Install Actions -->
        <div class="detail-install-bar">
          {#if selectedModlist.download_url}
            <button
              class="btn btn-primary btn-lg"
              onclick={() => selectedModlist && handleDownloadWabbajack(selectedModlist)}
              disabled={downloading}
            >
              {#if downloading}
                <span class="spinner spinner-sm"></span>
                Downloading...
              {:else}
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                  <polyline points="7 10 12 15 17 10" />
                  <line x1="12" y1="15" x2="12" y2="3" />
                </svg>
                Download .wabbajack
              {/if}
            </button>
            {#if downloadError}
              <p class="download-error">{downloadError}</p>
            {/if}
          {/if}
          <button class="btn btn-accent btn-lg" onclick={openLocalFile}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
            </svg>
            Open Local File
          </button>
        </div>
      </div>
    </div>

  {:else if parsedModlist}
    <!-- Parsed File View -->
    <div class="detail-view">
      <div class="detail-header">
        <button class="btn btn-ghost" onclick={backToBrowse}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M19 12H5" />
            <polyline points="12 19 5 12 12 5" />
          </svg>
          Back to Browse
        </button>
      </div>

      <div class="detail-content">
        <div class="detail-title-section">
          <div class="detail-title-row">
            <h2 class="detail-name">{parsedModlist.name || "Untitled Modlist"}</h2>
            <span class="game-badge">{parsedModlist.game_name}</span>
            {#if parsedModlist.is_nsfw}
              <span class="nsfw-badge">NSFW</span>
            {/if}
          </div>
          {#if parsedModlist.author}
            <p class="detail-author">by {parsedModlist.author}</p>
          {/if}
          {#if parsedModlist.version}
            <span class="detail-version">v{parsedModlist.version}</span>
          {/if}
        </div>

        {#if parsedModlist.description}
          <div class="detail-section">
            <h3 class="detail-section-title">Description</h3>
            <div class="detail-description rendered-markdown">{@html DOMPurify.sanitize(bbcodeToHtml(parsedModlist.description))}</div>
          </div>
        {/if}

        <!-- Stats -->
        <div class="detail-stats-bar">
          <div class="detail-stat">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
              <polyline points="7 10 12 15 17 10" />
              <line x1="12" y1="15" x2="12" y2="3" />
            </svg>
            <span class="detail-stat-value">{formatSize(parsedModlist.total_download_size)}</span>
            <span class="detail-stat-label">Download</span>
          </div>
          <div class="detail-stat">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
            </svg>
            <span class="detail-stat-value">{parsedModlist.archive_count.toLocaleString()}</span>
            <span class="detail-stat-label">Archives</span>
          </div>
          <div class="detail-stat">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" />
              <polyline points="13 2 13 9 20 9" />
            </svg>
            <span class="detail-stat-value">{parsedModlist.directive_count.toLocaleString()}</span>
            <span class="detail-stat-label">Directives</span>
          </div>
        </div>

        <!-- Archive Sources Breakdown -->
        <div class="detail-section">
          <h3 class="detail-section-title">Archive Sources</h3>
          <div class="source-breakdown">
            {#each archiveBreakdown(parsedModlist) as item}
              <div class="source-row">
                <div class="source-info">
                  <span class="source-dot" style="background: {sourceColor(item.source)}"></span>
                  <span class="source-name">{sourceLabel(item.source)}</span>
                </div>
                <div class="source-stats">
                  <span class="source-count">{item.count}</span>
                  <span class="source-size">{formatSize(item.size)}</span>
                </div>
              </div>
            {/each}
          </div>
        </div>

        <!-- Directive Breakdown -->
        {#if Object.keys(parsedModlist.directive_breakdown).length > 0}
          <div class="detail-section">
            <h3 class="detail-section-title">Directive Breakdown</h3>
            <div class="directive-breakdown">
              {#each Object.entries(parsedModlist.directive_breakdown).sort((a, b) => b[1] - a[1]) as [kind, count]}
                <div class="directive-row">
                  <span class="directive-kind">{kind}</span>
                  <span class="directive-count">{count.toLocaleString()}</span>
                </div>
              {/each}
            </div>
          </div>
        {/if}

        <!-- Compatibility Check -->
        {#if parsedModlist.game_name.includes("Skyrim") && $selectedGame}
          <div class="detail-section">
            <CompatibilityPanel gameId={$selectedGame.game_id} bottleName={$selectedGame.bottle_name} />
          </div>
        {/if}

        <!-- Install -->
        <div class="detail-install-bar">
          {#if !$selectedGame}
            <p class="install-note">Select a game in the sidebar to begin installation.</p>
          {:else if installing}
            <div class="wj-progress-section">
              <!-- Overall header with phase + speed + elapsed -->
              <div class="wj-progress-header">
                <span class="wj-phase-label">
                  {#if wjPhase === "preflight"}Preflight
                  {:else if wjPhase === "downloading"}Downloading
                  {:else if wjPhase === "extracting"}Extracting
                  {:else if wjPhase === "directives"}Processing
                  {:else if wjPhase === "deploying"}Deploying
                  {:else if wjPhase === "done"}Complete
                  {:else}Starting...
                  {/if}
                </span>
                {#if wjSpeed > 0}
                  <span class="wj-speed-badge">{SpeedTracker.formatSpeed(wjSpeed)}</span>
                {/if}
                {#if wjEta}
                  <span class="wj-eta-badge">{wjEta}</span>
                {/if}
                {#if wjElapsed && wjPhase !== "done"}
                  <span class="wj-elapsed-badge">{wjElapsed}</span>
                {/if}
                <button class="btn btn-ghost btn-sm" style="margin-left: auto;" onclick={handleCancelInstall}>Cancel</button>
              </div>

              <!-- Overall progress bar -->
              {#if wjOverallProgress > 0}
                <div class="wj-progress-bar-track wj-overall-track">
                  <div
                    class="wj-progress-bar-fill wj-overall-fill"
                    style="width: {wjOverallProgress}%"
                  ></div>
                </div>
                <div class="wj-overall-label">{wjOverallProgress}% overall</div>
              {/if}

              <!-- Phase progress bar -->
              {#if wjTotal > 0}
                <div class="wj-progress-bar-track">
                  <div
                    class="wj-progress-bar-fill"
                    style="width: {Math.min(100, (wjCurrent / wjTotal) * 100).toFixed(1)}%"
                  ></div>
                </div>
                <div class="wj-progress-counts">
                  <span>{wjCurrent.toLocaleString()} / {wjTotal.toLocaleString()}</span>
                  {#if wjTotalBytes > 0}
                    <span class="wj-bytes-progress">{SpeedTracker.formatBytes(wjBytesCompleted)} / {SpeedTracker.formatBytes(wjTotalBytes)}</span>
                  {/if}
                  {#if wjCurrentFile}
                    <span class="wj-progress-filename" title={wjCurrentFile}>{wjCurrentFile}</span>
                  {/if}
                </div>
              {/if}

              <!-- Per-archive status list (during extraction phase) -->
              {#if wjArchives.length > 0 && (wjPhase === "extracting")}
                <div class="wj-archive-list">
                  {#each wjArchives as archive (archive.index)}
                    <div class="wj-archive-item" class:wj-archive-done={archive.status === "extracted" || archive.status === "downloaded"} class:wj-archive-active={archive.status === "extracting" || archive.status === "downloading"} class:wj-archive-failed={archive.status === "failed"}>
                      <span class="wj-archive-status">
                        {#if archive.status === "extracting" || archive.status === "downloading"}
                          <span class="spinner-xs"></span>
                        {:else if archive.status === "extracted" || archive.status === "downloaded"}
                          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--system-green, #34C759)" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>
                        {:else if archive.status === "failed"}
                          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--system-red, #FF3B30)" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>
                        {:else}
                          <span class="wj-archive-pending-dot"></span>
                        {/if}
                      </span>
                      <span class="wj-archive-name" title={archive.name}>{archive.name}</span>
                      {#if archive.size > 0}
                        <span class="wj-archive-size">{SpeedTracker.formatBytes(archive.size)}</span>
                      {/if}
                    </div>
                  {/each}
                </div>
              {/if}

              {#if installStep}
                <p class="install-note">{installStep}</p>
              {/if}
            </div>
          {:else}
            <button
              class="btn btn-primary btn-lg"
              onclick={handleBeginInstall}
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                <polyline points="7 10 12 15 17 10" />
                <line x1="12" y1="15" x2="12" y2="3" />
              </svg>
              Begin Install
            </button>
            {#if installStep}
              <p class="install-note">{installStep}</p>
            {/if}
          {/if}
        </div>
      </div>
    </div>

    {#if showToolsPrompt && $selectedGame}
      <RequiredToolsPrompt
        tools={pendingTools}
        gameId={$selectedGame.game_id}
        bottleName={$selectedGame.bottle_name}
        oncontinue={proceedWithInstall}
        oncancel={() => { showToolsPrompt = false; }}
      />
    {/if}

  {:else if parsingFile}
    <div class="loading-container">
      <div class="loading-card">
        <div class="spinner"><div class="spinner-ring"></div></div>
        <div class="loading-text">
          <p class="loading-title">Parsing modlist</p>
          <p class="loading-detail">Reading .wabbajack file...</p>
        </div>
      </div>
    </div>

  {:else}
    <!-- Browse View -->
    <header class="page-header">
      <div class="header-text">
        <h2 class="page-title"><WabbajackLogo size={22} /> Wabbajack Lists</h2>
        <p class="page-subtitle">
          Browse Wabbajack modlists — curated, pre-configured mod setups
        </p>
      </div>
      <div class="header-right">
        <WebViewToggle
          bind:this={webviewToggle}
          url="https://www.wabbajack.org/gallery"
          onModeChange={(m) => viewMode = m}
        />
        <button class="btn btn-accent btn-sm" onclick={openLocalFile}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
          </svg>
          Open .wabbajack
        </button>
        {#if !loading && viewMode === "app"}
          <div class="stat-pill">
            <span class="stat-value">{filtered.length}</span>
            <span class="stat-label">{filtered.length === 1 ? "List" : "Lists"}</span>
          </div>
        {/if}
      </div>
    </header>

    {#if viewMode === "website"}
      <div class="webview-placeholder">
        <p class="webview-hint">Browsing Wabbajack Gallery directly. Switch to "In-App" to use built-in search and filters.</p>
      </div>
    {:else if loading}
      <div class="loading-container">
        <div class="loading-card">
          <div class="spinner"><div class="spinner-ring"></div></div>
          <div class="loading-text">
            <p class="loading-title">Fetching modlists</p>
            <p class="loading-detail">Loading gallery from Wabbajack repositories...</p>
          </div>
        </div>
      </div>
    {:else}
      <!-- Filters -->
      <div class="filters-bar">
        <input
          type="text"
          class="search-input"
          placeholder="Search modlists..."
          bind:value={searchQuery}
        />
        <select class="filter-select" bind:value={gameFilter}>
          <option value="all">All Games</option>
          {#each gameOptions as game}
            <option value={game}>{gameDomainDisplay(game)}</option>
          {/each}
        </select>
        <button
          class="nsfw-cycle-btn"
          class:nsfw-show={nsfwFilter === "show"}
          class:nsfw-only={nsfwFilter === "only"}
          onclick={() => nsfwFilter = cycleNsfwFilter(nsfwFilter)}
          title={nsfwFilter === "hide" ? "NSFW hidden" : nsfwFilter === "show" ? "NSFW included" : "NSFW only"}
        >
          <span class="nsfw-indicator">{nsfwIcon(nsfwFilter)}</span>
          {nsfwLabel(nsfwFilter)}
        </button>
        <button class="filter-toggle" onclick={() => showAdvancedFilters = !showAdvancedFilters}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="4" y1="6" x2="20" y2="6" />
            <line x1="8" y1="12" x2="20" y2="12" />
            <line x1="12" y1="18" x2="20" y2="18" />
          </svg>
          Filters {showAdvancedFilters ? '\u25B2' : '\u25BC'}
          {#if activeFilterCount > 0}<span class="filter-badge">{activeFilterCount}</span>{/if}
        </button>
        <div class="sort-group">
          <select class="filter-select" bind:value={sortField}>
            <option value="title">Sort: Name</option>
            <option value="author">Sort: Author</option>
            <option value="download_size">Sort: Download Size</option>
            <option value="install_size">Sort: Install Size</option>
          </select>
          <button
            class="sort-direction-btn"
            onclick={() => sortDirection = sortDirection === "asc" ? "desc" : "asc"}
            title={sortDirection === "asc" ? "Ascending" : "Descending"}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              {#if sortDirection === "asc"}
                <path d="M12 5v14M5 12l7-7 7 7" />
              {:else}
                <path d="M12 5v14M5 12l7 7 7-7" />
              {/if}
            </svg>
          </button>
        </div>
      </div>

      {#if showAdvancedFilters}
        <div class="advanced-filters">
          <!-- Install Size -->
          <div class="filter-section">
            <span class="filter-label">Install Size</span>
            <div class="filter-pills">
              <button class="filter-pill" class:active={maxInstallSize === null} onclick={() => maxInstallSize = null}>Any</button>
              <button class="filter-pill" class:active={maxInstallSize === 50_000_000_000} onclick={() => maxInstallSize = 50_000_000_000}>&lt; 50 GB</button>
              <button class="filter-pill" class:active={maxInstallSize === 100_000_000_000} onclick={() => maxInstallSize = 100_000_000_000}>&lt; 100 GB</button>
              <button class="filter-pill" class:active={maxInstallSize === 200_000_000_000} onclick={() => maxInstallSize = 200_000_000_000}>&lt; 200 GB</button>
            </div>
          </div>

          <!-- Tags -->
          {#if availableTags.length > 0}
            <div class="filter-section">
              <span class="filter-label">Tags</span>
              <div class="filter-pills">
                {#each availableTags as tag}
                  <button
                    class="filter-pill"
                    class:active={tagFilter.includes(tag)}
                    onclick={() => {
                      if (tagFilter.includes(tag)) tagFilter = tagFilter.filter(t => t !== tag);
                      else tagFilter = [...tagFilter, tag];
                    }}
                  >{tag}</button>
                {/each}
              </div>
            </div>
          {/if}
        </div>
      {/if}

      <!-- Active filter chips -->
      {#if activeFilterCount > 0}
        <div class="active-filters">
          {#if maxInstallSize !== null}
            <span class="filter-chip">
              Install &lt; {maxInstallSize / 1_000_000_000} GB
              <button onclick={() => maxInstallSize = null} title="Remove filter">&times;</button>
            </span>
          {/if}
          {#each tagFilter as tag}
            <span class="filter-chip">
              {tag}
              <button onclick={() => tagFilter = tagFilter.filter(t => t !== tag)} title="Remove filter">&times;</button>
            </span>
          {/each}
        </div>
      {/if}

      {#if filtered.length === 0}
        <div class="empty-state">
          <p class="empty-title">No modlists found</p>
          <p class="empty-detail">
            {#if searchQuery || gameFilter !== "all" || activeFilterCount > 0}
              Try adjusting your search or filters.
            {:else}
              No modlists are currently available.
            {/if}
          </p>
        </div>
      {:else}
        <div class="modlist-grid">
          {#each filtered as modlist, i}
            <div class="modlist-card" style="animation-delay: {Math.min(i, 20) * 30}ms">
              {#if modlist.image_url}
                <div class="card-image">
                  <img src={modlist.image_url} alt={modlist.title} loading="lazy" />
                </div>
              {/if}
              <div class="card-body">
                <div class="card-top">
                  <span class="game-badge">{gameDomainDisplay(modlist.game)}</span>
                  {#if modlist.nsfw}
                    <span class="nsfw-badge">NSFW</span>
                  {/if}
                  <span class="version-badge">v{modlist.version}</span>
                </div>

                <h3 class="card-title">{modlist.title}</h3>
                <p class="card-author">by {modlist.author}</p>

                {#if modlist.description}
                  <p class="card-desc">{modlist.description}</p>
                {/if}

                {#if modlist.tags.length > 0}
                  <div class="card-tags">
                    {#each modlist.tags.slice(0, 5) as tag}
                      <span class="tag">{tag}</span>
                    {/each}
                  </div>
                {/if}

                <div class="card-stats">
                  <div class="stat-item">
                    <span class="stat-num">{formatSize(modlist.download_size)}</span>
                    <span class="stat-lbl">Download</span>
                  </div>
                  <div class="stat-item">
                    <span class="stat-num">{formatSize(modlist.install_size)}</span>
                    <span class="stat-lbl">Install</span>
                  </div>
                  <div class="stat-item">
                    <span class="stat-num">{modlist.archive_count.toLocaleString()}</span>
                    <span class="stat-lbl">Archives</span>
                  </div>
                  <div class="stat-item">
                    <span class="stat-num">{modlist.file_count.toLocaleString()}</span>
                    <span class="stat-lbl">Files</span>
                  </div>
                </div>

                <div class="card-actions">
                  <button
                    class="btn btn-accent btn-sm"
                    onclick={() => viewModlistDetail(modlist)}
                  >
                    View Details
                  </button>
                </div>
              </div>
            </div>
          {/each}
        </div>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .modlists-page {
    padding: var(--space-2) 0 var(--space-12) 0;
  }

  .page-header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    margin-bottom: var(--space-8);
    padding-bottom: var(--space-4);
    border-bottom: 1px solid var(--separator);
  }

  .page-title {
    font-size: 28px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.025em;
  }

  .page-subtitle {
    font-size: 14px;
    color: var(--text-secondary);
    margin-top: var(--space-1);
  }

  .stat-pill {
    display: flex;
    align-items: baseline;
    gap: var(--space-1);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: var(--space-2) var(--space-4);
  }

  .stat-value {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .stat-label {
    font-size: 12px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  /* Loading */
  .loading-container {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 280px;
  }

  .loading-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-6);
    padding: var(--space-12) var(--space-10);
  }

  .spinner { width: 36px; height: 36px; }

  .spinner-ring {
    width: 100%;
    height: 100%;
    border: 2.5px solid var(--separator);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.9s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  }

  @keyframes spin { to { transform: rotate(360deg); } }

  .loading-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    text-align: center;
  }

  .loading-detail {
    font-size: 13px;
    color: var(--text-tertiary);
    text-align: center;
    margin-top: var(--space-1);
  }

  /* Filters */
  .filters-bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-6);
  }

  .search-input {
    flex: 1;
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    color: var(--text-primary);
    font-size: 14px;
    outline: none;
    transition: border-color var(--duration-fast) var(--ease);
  }

  .search-input:focus {
    border-color: var(--system-accent);
  }

  .search-input::placeholder {
    color: var(--text-tertiary);
  }

  .filter-select {
    padding: var(--space-2) var(--space-3);
    background: var(--bg-tertiary);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    color: var(--text-primary);
    font-size: 13px;
    outline: none;
    cursor: pointer;
    min-width: 140px;
  }

  .filter-select:focus {
    border-color: var(--system-accent);
  }

  .sort-group {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin-left: auto;
  }

  .sort-direction-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: var(--radius-sm);
    background: var(--surface);
    border: 1px solid var(--separator);
    color: var(--text-secondary);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .sort-direction-btn:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  /* Advanced Filters */
  .filter-toggle {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    color: var(--text-secondary);
    font-size: 12px;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }

  .filter-toggle:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .filter-badge {
    background: var(--accent, var(--system-accent));
    color: var(--accent-on, #fff);
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 10px;
    font-weight: 600;
  }

  .advanced-filters {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    background: var(--surface-subtle, var(--bg-secondary));
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    margin-bottom: var(--space-4);
  }

  .filter-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 140px;
  }

  .filter-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .filter-pills {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }

  .filter-pill {
    padding: 3px 10px;
    background: var(--bg-tertiary);
    border: 1px solid var(--separator);
    border-radius: 12px;
    color: var(--text-secondary);
    font-size: 11px;
    cursor: pointer;
    transition: all 0.15s;
    white-space: nowrap;
  }

  .filter-pill:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .filter-pill.active {
    background: var(--system-accent-subtle);
    border-color: var(--system-accent);
    color: var(--system-accent);
    font-weight: 500;
  }

  .active-filters {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-bottom: var(--space-4);
  }

  .filter-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    background: var(--system-accent-subtle);
    border: 1px solid var(--system-accent-muted, rgba(0, 122, 255, 0.25));
    border-radius: 10px;
    color: var(--system-accent);
    font-size: 11px;
    font-weight: 500;
  }

  .filter-chip button {
    display: flex;
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
    padding: 0;
    opacity: 0.7;
    font-size: 13px;
    line-height: 1;
  }

  .filter-chip button:hover {
    opacity: 1;
  }

  /* Grid */
  .modlist-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: var(--space-4);
  }

  /* Cards */
  .modlist-card {
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    overflow: hidden;
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    transition: border-color var(--duration-fast) var(--ease),
                box-shadow var(--duration-fast) var(--ease);
    animation: cardFadeIn var(--duration-slow) var(--ease) both;
    display: flex;
    flex-direction: column;
  }

  .modlist-card:hover {
    border-color: var(--separator);
  }

  @keyframes cardFadeIn {
    from { opacity: 0; transform: translateY(6px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .card-image {
    width: 100%;
    height: 140px;
    overflow: hidden;
    background: var(--bg-secondary);
  }

  .card-image img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .card-body {
    padding: var(--space-4) var(--space-5);
    display: flex;
    flex-direction: column;
    flex: 1;
  }

  .card-top {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
    flex-wrap: wrap;
  }

  .game-badge {
    font-size: 11px;
    font-weight: 600;
    color: var(--system-accent);
    background: var(--system-accent-subtle);
    padding: 1px var(--space-2);
    border-radius: var(--radius-sm);
  }

  .nsfw-badge {
    font-size: 10px;
    font-weight: 700;
    color: var(--red);
    background: var(--red-subtle);
    padding: 1px var(--space-2);
    border-radius: var(--radius-sm);
  }

  .version-badge {
    font-size: 10px;
    font-weight: 500;
    color: var(--text-tertiary);
    background: var(--surface);
    padding: 1px var(--space-2);
    border-radius: var(--radius-sm);
  }

  .card-title {
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
    line-height: 1.3;
    margin-bottom: 2px;
  }

  .card-author {
    font-size: 12px;
    color: var(--text-secondary);
    margin-bottom: var(--space-2);
  }

  .card-desc {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.5;
    margin-bottom: var(--space-3);
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .card-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-bottom: var(--space-3);
  }

  .tag {
    font-size: 10px;
    font-weight: 500;
    color: var(--text-secondary);
    background: var(--surface);
    padding: 1px 6px;
    border-radius: var(--radius-sm);
  }

  .card-stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--space-2);
    padding: var(--space-3) 0;
    border-top: 1px solid var(--separator);
    margin-bottom: var(--space-3);
  }

  .stat-item {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
  }

  .stat-num {
    font-size: 12px;
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .stat-lbl {
    font-size: 10px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  .card-actions {
    display: flex;
    gap: var(--space-2);
    margin-top: auto;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    border-radius: var(--radius);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .btn-sm {
    padding: var(--space-1) var(--space-3);
    font-size: 12px;
    min-height: 28px;
  }

  .btn-lg {
    padding: var(--space-3) var(--space-6);
    font-size: 14px;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-accent {
    background: var(--system-accent);
    color: #fff;
    flex: 1;
  }

  .btn-accent:hover:not(:disabled) {
    filter: brightness(1.1);
  }

  .btn-primary {
    background: var(--system-accent);
    color: var(--system-accent-on);
    padding: var(--space-2) var(--space-5);
    border-radius: var(--radius);
  }

  .btn-primary:hover:not(:disabled) {
    background: var(--system-accent-hover);
    box-shadow: 0 1px 6px rgba(0, 122, 255, 0.25);
  }

  .btn-ghost {
    background: transparent;
    color: var(--text-secondary);
    padding: var(--space-2) var(--space-3);
    font-size: 13px;
    font-weight: 500;
  }

  .btn-ghost:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  /* Empty state */
  .empty-state {
    text-align: center;
    padding: var(--space-12) var(--space-8);
    border: 1px dashed var(--separator);
    border-radius: var(--radius-lg);
    background: var(--surface-subtle);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
  }

  .empty-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-secondary);
    margin-bottom: var(--space-2);
  }

  .empty-detail {
    font-size: 13px;
    color: var(--text-tertiary);
    max-width: 360px;
    margin: 0 auto;
    line-height: 1.55;
  }

  /* ---- Detail View ---- */

  .detail-view {
    animation: fadeIn var(--duration-normal) var(--ease);
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .detail-header {
    margin-bottom: var(--space-4);
  }

  .detail-content {
    max-width: 800px;
  }

  .detail-title-section {
    margin-bottom: var(--space-6);
    padding-bottom: var(--space-4);
    border-bottom: 1px solid var(--separator);
  }

  .detail-title-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
    margin-bottom: var(--space-1);
  }

  .detail-name {
    font-size: 26px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.02em;
  }

  .detail-author {
    font-size: 14px;
    color: var(--text-secondary);
    margin-bottom: var(--space-1);
  }

  .detail-version {
    font-size: 12px;
    color: var(--text-tertiary);
    background: var(--surface);
    padding: 2px var(--space-2);
    border-radius: var(--radius-sm);
  }

  /* Stats Bar */
  .detail-stats-bar {
    display: flex;
    gap: var(--space-6);
    padding: var(--space-4) var(--space-5);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    margin-bottom: var(--space-6);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
  }

  .detail-stat {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-tertiary);
  }

  .detail-stat svg {
    opacity: 0.6;
  }

  .detail-stat-value {
    font-size: 14px;
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .detail-stat-label {
    font-size: 12px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  /* Detail Sections */
  .detail-section {
    margin-bottom: var(--space-6);
  }

  .detail-section-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-bottom: var(--space-3);
  }

  .detail-description {
    font-size: 14px;
    line-height: 1.7;
    color: var(--text-secondary);
  }

  .detail-tags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .detail-tags .tag {
    font-size: 12px;
    padding: var(--space-1) var(--space-3);
  }

  .readme-loading {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-6);
    color: var(--text-tertiary);
    font-size: 13px;
  }

  .spinner-sm-ring {
    width: 18px;
    height: 18px;
    border: 2px solid var(--separator);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.9s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  }

  .detail-no-readme {
    font-size: 13px;
    color: var(--text-tertiary);
    font-style: italic;
  }

  .detail-install-bar {
    padding: var(--space-6) 0;
    border-top: 1px solid var(--separator);
    margin-top: var(--space-4);
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    align-items: center;
  }

  .install-note {
    font-size: 13px;
    color: var(--text-tertiary);
    flex-basis: 100%;
    margin-top: var(--space-1);
  }

  .wj-progress-section {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    animation: glass-fade-in var(--duration) var(--ease-out);
  }

  .wj-progress-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .wj-phase-label {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .wj-progress-bar-track {
    width: 100%;
    height: 6px;
    background: var(--bg-tertiary);
    border-radius: 3px;
    overflow: hidden;
  }

  .wj-progress-bar-fill {
    height: 100%;
    background: var(--system-accent);
    border-radius: 3px;
    transition: width 0.2s ease;
    position: relative;
    overflow: hidden;
  }

  .wj-progress-bar-fill::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(
      90deg,
      transparent 0%,
      rgba(255, 255, 255, 0.3) 45%,
      rgba(255, 255, 255, 0.4) 50%,
      rgba(255, 255, 255, 0.3) 55%,
      transparent 100%
    );
    animation: glass-progress-shimmer 2s var(--ease) infinite;
  }

  .wj-progress-counts {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .wj-progress-filename {
    max-width: 300px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-tertiary);
  }

  .wj-speed-badge,
  .wj-eta-badge,
  .wj-elapsed-badge {
    font-size: 11px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 4px;
    font-variant-numeric: tabular-nums;
  }

  .wj-speed-badge {
    background: color-mix(in srgb, var(--system-accent, #007AFF) 15%, transparent);
    color: var(--system-accent, #007AFF);
  }

  .wj-eta-badge {
    background: color-mix(in srgb, var(--text-secondary) 10%, transparent);
    color: var(--text-secondary);
  }

  .wj-elapsed-badge {
    background: color-mix(in srgb, var(--text-tertiary) 10%, transparent);
    color: var(--text-tertiary);
  }

  .wj-overall-track {
    height: 3px;
    margin-bottom: 2px;
  }

  .wj-overall-fill {
    background: var(--system-green, #34C759);
  }

  .wj-overall-label {
    font-size: 10px;
    color: var(--text-tertiary);
    text-align: right;
    margin-bottom: var(--space-2);
    font-variant-numeric: tabular-nums;
  }

  .wj-bytes-progress {
    font-size: 11px;
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .wj-archive-list {
    max-height: 200px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: var(--space-2);
    padding: var(--space-2);
    background: var(--surface-secondary, rgba(255,255,255,0.03));
    border-radius: 6px;
  }

  .wj-archive-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 3px 6px;
    border-radius: 4px;
    font-size: 12px;
    transition: opacity 0.2s;
  }

  .wj-archive-done {
    opacity: 0.5;
  }

  .wj-archive-active {
    background: color-mix(in srgb, var(--system-accent, #007AFF) 8%, transparent);
  }

  .wj-archive-failed {
    background: color-mix(in srgb, var(--system-red, #FF3B30) 8%, transparent);
  }

  .wj-archive-status {
    width: 16px;
    height: 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .wj-archive-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-primary);
  }

  .wj-archive-size {
    font-size: 11px;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .wj-archive-pending-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text-tertiary);
    opacity: 0.4;
  }

  .spinner-xs {
    width: 12px;
    height: 12px;
    border: 2px solid var(--surface-hover, rgba(255,255,255,0.1));
    border-top-color: var(--system-accent, #007AFF);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  .header-right {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  /* Source breakdown */
  .source-breakdown {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .source-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
  }

  .source-info {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .source-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .source-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
  }

  .source-stats {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .source-count {
    font-size: 13px;
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .source-size {
    font-size: 12px;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
    min-width: 60px;
    text-align: right;
  }

  /* Directive breakdown */
  .directive-breakdown {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .directive-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-1) var(--space-3);
  }

  .directive-kind {
    font-size: 13px;
    color: var(--text-secondary);
  }

  .directive-count {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .download-error {
    color: #ef4444;
    font-size: 13px;
    margin-top: 8px;
    padding: 8px 12px;
    background: rgba(239, 68, 68, 0.1);
    border-radius: 6px;
    border: 1px solid rgba(239, 68, 68, 0.2);
  }

  /* ---- Webview Placeholder ---- */

  .webview-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 120px;
    padding: var(--space-8);
  }

  .webview-hint {
    font-size: 13px;
    color: var(--text-tertiary);
    text-align: center;
  }

  /* ---- NSFW 3-State Toggle ---- */

  .nsfw-cycle-btn {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-2) var(--space-3);
    background: var(--bg-tertiary);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    white-space: nowrap;
  }

  .nsfw-cycle-btn:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .nsfw-cycle-btn.nsfw-show {
    background: rgba(255, 159, 10, 0.1);
    border-color: rgba(255, 159, 10, 0.3);
    color: #ff9f0a;
  }

  .nsfw-cycle-btn.nsfw-only {
    background: rgba(255, 69, 58, 0.1);
    border-color: rgba(255, 69, 58, 0.3);
    color: #ff453a;
  }

  .nsfw-indicator {
    font-size: 11px;
    font-weight: 700;
    width: 14px;
    text-align: center;
  }

  /* ---- Resume Banner ---- */

  .resume-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    background: rgba(255, 159, 10, 0.1);
    border: 1px solid rgba(255, 159, 10, 0.3);
    border-radius: var(--radius-md);
    margin-bottom: var(--space-4);
  }

  .resume-info {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .resume-icon {
    font-size: 20px;
  }

  .resume-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .resume-title {
    font-weight: 600;
    color: var(--text-primary);
    font-size: 13px;
  }

  .resume-detail {
    font-size: 12px;
    color: var(--text-secondary);
  }

  .resume-actions {
    display: flex;
    gap: var(--space-2);
    flex-shrink: 0;
  }
</style>
