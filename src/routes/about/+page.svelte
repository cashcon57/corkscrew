<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { appVersion } from "$lib/stores";
</script>

<div class="about-page">
  <!-- App Identity -->
  <div class="app-identity">
    <img
      src="/corkscrew-icon.png"
      alt="Corkscrew app icon"
      class="app-icon"
      width="80"
      height="80"
    />
    <h1 class="app-name">Corkscrew</h1>
    <span class="app-version">v{$appVersion}</span>
    <p class="app-tagline">A native mod manager for Wine games</p>
    <button
      class="kofi-btn"
      onclick={() => openUrl("https://ko-fi.com/cash508287")}
      title="Support Corkscrew on Ko-fi"
    >
      <svg class="kofi-icon" width="16" height="16" viewBox="0 0 24 24" fill="none">
        <path d="M12 4.5C7 4.5 4 8 4 11c0 3.5 3.5 7.5 8 11 4.5-3.5 8-7.5 8-11 0-3-3-6.5-8-6.5z" fill="#FF5E5B" />
      </svg>
      <span>Support this Project</span>
    </button>
  </div>

  <!-- Project Info -->
  <div class="section">
    <h2 class="section-title">Project</h2>
    <div class="section-card">
      <div class="card-row">
        <span class="row-label">Author</span>
        <span class="row-value">cashconway</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <span class="row-label">License</span>
        <span class="row-value">GPL-3.0-or-later</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <span class="row-label">Platform</span>
        <span class="row-value">macOS / Linux</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <span class="row-label">GitHub</span>
        <span class="row-value">
          <button class="link-btn" onclick={() => openUrl("https://github.com/cashcon57/corkscrew")}>
            github.com/cashcon57/corkscrew
          </button>
        </span>
      </div>
    </div>
  </div>

  <!-- Built With -->
  <div class="section">
    <h2 class="section-title">Built With</h2>
    <div class="section-card">
      <div class="card-row">
        <span class="row-label">Tauri</span>
        <span class="row-value">v2</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <span class="row-label">Svelte</span>
        <span class="row-value">v5</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <span class="row-label">Rust</span>
        <span class="row-value">2021 edition</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <span class="row-label">SQLite</span>
        <span class="row-value">via rusqlite</span>
      </div>
    </div>
  </div>

  <!-- Projects We Build Upon -->
  <div class="section">
    <h2 class="section-title">Projects We Build Upon</h2>
    <p class="section-subtitle">
      Corkscrew stands on the shoulders of these open source projects.
      We are deeply grateful for their work.
    </p>
    <div class="section-card">
      <button class="credit-row" onclick={() => openUrl("https://github.com/loot/libloot")}>
        <div class="credit-info">
          <span class="credit-name">LOOT / libloot</span>
          <span class="credit-desc">
            Plugin sorting engine powering Corkscrew's automatic load order management. Created by WrinklyNinja.
          </span>
        </div>
        <span class="credit-license">GPL-3.0</span>
      </button>
      <div class="card-divider"></div>
      <button class="credit-row" onclick={() => openUrl("https://github.com/wabbajack-tools/wabbajack")}>
        <div class="credit-info">
          <span class="credit-name">Wabbajack</span>
          <span class="credit-desc">
            Pioneered automated modlist installation and the Stock Game approach. Corkscrew browses Wabbajack's modlist gallery.
          </span>
        </div>
        <span class="credit-license">GPL-3.0</span>
      </button>
      <div class="card-divider"></div>
      <button class="credit-row" onclick={() => openUrl("https://github.com/Nexus-Mods/Vortex")}>
        <div class="credit-info">
          <span class="credit-name">Vortex</span>
          <span class="credit-desc">
            Deployment model, conflict resolution patterns, and Nexus Collections format informed Corkscrew's staging engine.
          </span>
        </div>
        <span class="credit-license">GPL-3.0</span>
      </button>
      <div class="card-divider"></div>
      <button class="credit-row" onclick={() => openUrl("https://github.com/ModOrganizer2/modorganizer")}>
        <div class="credit-info">
          <span class="credit-name">Mod Organizer 2</span>
          <span class="credit-desc">
            Virtual filesystem concept and profile system inspired Corkscrew's staging/deploy architecture and mod profiles.
          </span>
        </div>
        <span class="credit-license">GPL-3.0</span>
      </button>
    </div>
  </div>

  <!-- Communities & Services -->
  <div class="section">
    <h2 class="section-title">Communities & Services</h2>
    <div class="section-card">
      <button class="credit-row" onclick={() => openUrl("https://skse.silverlock.org/")}>
        <div class="credit-info">
          <span class="credit-name">SKSE Team</span>
          <span class="credit-desc">Skyrim Script Extender — essential for most Skyrim mods.</span>
        </div>
      </button>
      <div class="card-divider"></div>
      <button class="credit-row" onclick={() => openUrl("https://www.winehq.org/")}>
        <div class="credit-info">
          <span class="credit-name">Wine Project</span>
          <span class="credit-desc">The foundation that makes running Windows games on macOS and Linux possible.</span>
        </div>
      </button>
      <div class="card-divider"></div>
      <button class="credit-row" onclick={() => openUrl("https://www.codeweavers.com/crossover")}>
        <div class="credit-info">
          <span class="credit-name">CrossOver</span>
          <span class="credit-desc">A polished Wine implementation by CodeWeavers and major Wine contributor.</span>
        </div>
      </button>
      <div class="card-divider"></div>
      <button class="credit-row" onclick={() => openUrl("https://www.nexusmods.com/")}>
        <div class="credit-info">
          <span class="credit-name">Nexus Mods</span>
          <span class="credit-desc">The modding community, mod hosting platform, and API that mod managers depend on.</span>
        </div>
      </button>
      <div class="card-divider"></div>
      <button class="credit-row" onclick={() => openUrl("https://fomod-docs.readthedocs.io/")}>
        <div class="credit-info">
          <span class="credit-name">FOMOD Standard</span>
          <span class="credit-desc">The XML-based mod installer format used by many mod authors.</span>
        </div>
      </button>
      <div class="card-divider"></div>
      <button class="credit-row" onclick={() => openUrl("https://github.com/Omni-guides/Jackify")}>
        <div class="credit-info">
          <span class="credit-name">Jackify</span>
          <span class="credit-desc">Pioneered Wabbajack modlist installation on Linux and SteamOS/Steam Deck.</span>
        </div>
      </button>
    </div>
  </div>

  <!-- Libraries -->
  <div class="section">
    <h2 class="section-title">Key Libraries</h2>
    <div class="section-card">
      <div class="card-row">
        <span class="row-label">Tauri</span>
        <span class="row-value">Cross-platform app framework</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <span class="row-label">Svelte</span>
        <span class="row-value">Frontend framework</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <span class="row-label">libloot</span>
        <span class="row-value">Plugin sorting (Rust)</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <span class="row-label">rusqlite</span>
        <span class="row-value">SQLite bindings</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <span class="row-label">quick-xml</span>
        <span class="row-value">FOMOD XML parsing</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <span class="row-label">reqwest</span>
        <span class="row-value">HTTP client</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <span class="row-label">sevenz-rust</span>
        <span class="row-value">7z archive extraction</span>
      </div>
    </div>
  </div>

  <!-- Footer -->
  <p class="about-footer">Made with care for the modding community</p>
</div>

<style>
  .about-page {
    width: 100%;
    max-width: 860px;
    margin: 0 auto;
    padding: var(--space-8) var(--space-6);
  }

  /* --- App Identity --- */

  .app-identity {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    margin-bottom: var(--space-8);
  }

  .app-icon {
    width: 80px;
    height: 80px;
    border-radius: var(--radius-xl);
    margin-bottom: var(--space-4);
    box-shadow: var(--shadow);
  }

  .app-name {
    font-size: 28px;
    font-weight: 700;
    letter-spacing: -0.025em;
    color: var(--text-primary);
    line-height: 1.2;
  }

  .app-version {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-tertiary);
    margin-top: var(--space-1);
  }

  .app-tagline {
    font-size: 14px;
    color: var(--text-secondary);
    margin-top: var(--space-2);
    line-height: 1.45;
  }

  .kofi-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 18px;
    margin-top: var(--space-4);
    background: rgba(255, 94, 91, 0.1);
    border: 1px solid rgba(255, 94, 91, 0.2);
    border-radius: var(--radius);
    color: #FF5E5B;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    white-space: nowrap;
  }

  .kofi-btn:hover {
    background: rgba(255, 94, 91, 0.18);
    border-color: rgba(255, 94, 91, 0.35);
  }

  .kofi-icon {
    flex-shrink: 0;
  }

  /* --- Sections --- */

  .section {
    margin-bottom: var(--space-6);
  }

  .section-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.02em;
    padding: 0 var(--space-4);
    margin-bottom: var(--space-2);
  }

  .section-subtitle {
    font-size: 12px;
    color: var(--text-tertiary);
    padding: 0 var(--space-4);
    margin-bottom: var(--space-3);
    line-height: 1.5;
  }

  .section-card {
    background: var(--bg-grouped-secondary);
    border-radius: var(--radius-lg);
    overflow: hidden;
    box-shadow: var(--glass-edge-shadow);
  }

  /* --- Card rows --- */

  .card-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3) var(--space-4);
  }

  .card-divider {
    height: 1px;
    background: var(--separator);
    margin-left: var(--space-4);
  }

  .row-label {
    font-size: 13px;
    font-weight: 400;
    color: var(--text-primary);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .row-value {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-secondary);
    text-align: right;
  }

  .link-btn {
    font-size: 13px;
    font-weight: 500;
    color: var(--system-accent);
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
    text-align: right;
  }

  .link-btn:hover {
    text-decoration: underline;
  }

  /* --- Credit rows (clickable) --- */

  .credit-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    width: 100%;
    text-align: left;
    cursor: pointer;
    background: none;
    border: none;
    transition: background var(--duration-fast) var(--ease);
  }

  .credit-row:hover {
    background: var(--surface-hover);
  }

  .credit-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .credit-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .credit-desc {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.4;
  }

  .credit-license {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-tertiary);
    background: rgba(255, 255, 255, 0.06);
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    white-space: nowrap;
    flex-shrink: 0;
  }

  /* --- Footer --- */

  .about-footer {
    text-align: center;
    font-size: 12px;
    color: var(--text-tertiary);
    margin-top: var(--space-8);
    padding-bottom: var(--space-4);
  }
</style>
