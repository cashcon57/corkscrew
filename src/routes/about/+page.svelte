<script lang="ts">
  // No reactive state needed — this is a static informational page.
</script>

<div class="about-page">
  <!-- App Identity -->
  <div class="app-identity">
    <img
      src="/icon-128.png"
      alt="Corkscrew app icon"
      class="app-icon"
      width="80"
      height="80"
    />
    <h1 class="app-name">Corkscrew</h1>
    <span class="app-version">v0.1.0</span>
    <p class="app-tagline">A native mod manager for Wine games</p>
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
          <a
            href="https://github.com/cashcon57/corkscrew"
            target="_blank"
            rel="noopener noreferrer"
          >github.com/cashcon57/corkscrew</a>
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
        <span class="row-value">
          <span class="badge">v2</span>
        </span>
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
    </div>
  </div>

  <!-- Acknowledgments -->
  <div class="section">
    <h2 class="section-title">Acknowledgments</h2>
    <div class="section-card">
      <div class="card-row">
        <span class="row-label">SKSE Team</span>
        <span class="row-value">Skyrim Script Extender</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <span class="row-label">Wine Project</span>
        <span class="row-value">Windows compatibility layer</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <span class="row-label">CodeWeavers</span>
        <span class="row-value">CrossOver</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <span class="row-label">Nexus Mods</span>
        <span class="row-value">Mod hosting platform</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <span class="row-label">Wabbajack</span>
        <span class="row-value">Automated modlist installer</span>
      </div>
    </div>
  </div>

  <!-- Footer -->
  <p class="about-footer">Made with care for the modding community</p>
</div>

<style>
  .about-page {
    max-width: 540px;
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

  .section-card {
    background: var(--bg-grouped-secondary);
    border-radius: var(--radius-lg);
    overflow: hidden;
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

  .row-value a {
    color: var(--accent);
    text-decoration: none;
  }

  .row-value a:hover {
    text-decoration: underline;
  }

  /* --- Badge --- */

  .badge {
    display: inline-block;
    padding: 1px var(--space-2);
    font-size: 11px;
    font-weight: 600;
    color: var(--accent);
    background: var(--accent-subtle);
    border-radius: var(--radius-sm);
    letter-spacing: 0.01em;
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
