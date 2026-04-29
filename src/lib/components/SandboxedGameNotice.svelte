<script lang="ts">
  let { gameName = "this game" }: { gameName?: string } = $props();
</script>

<div class="notice">
  <div class="icon">&#x26A0;</div>
  <div class="content">
    <h3>This game is sandboxed</h3>
    <p>
      Mac App Store apps and macOS system apps run in a sandbox that prevents file modifications.
      Corkscrew cannot mod {gameName}.
    </p>
    <p class="alt">
      If this game is also available on Steam, GOG, or directly from the developer, install that
      version to enable modding.
    </p>
    <button class="learn" onclick={() => console.log("TODO: open docs")}>Learn more</button>
  </div>
</div>

<style>
  .notice {
    display: flex;
    gap: 16px;
    padding: 24px;
    border-radius: 12px;
    background: var(--yellow-subtle);
    border: 1px solid var(--yellow);
    align-items: flex-start;
  }
  .icon {
    font-size: 24px;
    color: var(--yellow);
    flex-shrink: 0;
    line-height: 1;
  }
  h3 {
    font-size: 16px;
    font-weight: 600;
    margin: 0 0 8px;
    color: var(--text-primary);
  }
  p {
    margin: 0 0 8px;
    color: var(--text-secondary);
    font-size: 14px;
    line-height: 1.5;
  }
  p.alt {
    color: var(--text-tertiary);
  }
  .learn {
    background: transparent;
    border: 1px solid var(--separator);
    color: var(--text-secondary);
    padding: 6px 12px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
    margin-top: 8px;
  }
  .learn:hover {
    color: var(--text-primary);
    border-color: var(--text-secondary);
  }
</style>
