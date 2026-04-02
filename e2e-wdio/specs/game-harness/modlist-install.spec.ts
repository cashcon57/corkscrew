/**
 * Game Harness: Modlist Install + Launch + Crash Loop
 *
 * This is NOT a standard E2E test — it's a game testing harness that:
 * 1. Installs a Wabbajack modlist (Halgari's Helper or similar small list)
 * 2. Launches the game via Corkscrew
 * 3. Monitors for crashes
 * 4. Logs crash details from the crash analyzer
 * 5. Optionally toggles mods/patches and retries
 *
 * Run manually: npx wdio run wdio.conf.ts --spec specs/game-harness/
 * Requires: Real game installed, WJ modlist file accessible
 *
 * Environment variables:
 *   GAME_HARNESS=1              - Enable this spec (skipped otherwise)
 *   HARNESS_MODLIST_PATH        - Path to .wabbajack file
 *   HARNESS_MAX_ITERATIONS=5    - Max crash-retry loops
 *   HARNESS_LAUNCH_TIMEOUT=60   - Seconds to wait for game launch
 */
const ENABLED = process.env.GAME_HARNESS === "1";
const MODLIST_PATH = process.env.HARNESS_MODLIST_PATH || "";
const MAX_ITERATIONS = parseInt(process.env.HARNESS_MAX_ITERATIONS || "5", 10);
const LAUNCH_TIMEOUT = parseInt(process.env.HARNESS_LAUNCH_TIMEOUT || "60", 10) * 1000;

describe("Game Harness: Modlist Install & Launch Loop", () => {
  before(async function () {
    if (!ENABLED) {
      console.log("[harness] Skipped — set GAME_HARNESS=1 to enable");
      this.skip();
      return;
    }

    const shell = await $(".app-shell");
    await shell.waitForExist({ timeout: 20_000 });
  });

  it("navigates to Wabbajack modlists page", async function () {
    if (!ENABLED) this.skip();

    // Find and click the Wabbajack / Modlists nav item
    const navItems = await $$(".nav-item");
    let found = false;
    for (const nav of navItems) {
      const text = await nav.getText();
      if (text.includes("Modlist") || text.includes("Wabbajack") || text.includes("Discover")) {
        await nav.click();
        await browser.pause(2_000);
        found = true;
        break;
      }
    }
    expect(found).toBe(true);
  });

  it("installs modlist and waits for completion", async function () {
    if (!ENABLED || !MODLIST_PATH) {
      console.log("[harness] No HARNESS_MODLIST_PATH set — skipping install");
      this.skip();
      return;
    }

    // This is a long-running operation — set generous timeout
    this.timeout(600_000); // 10 minutes

    // The actual install flow depends on whether we're using the WJ gallery
    // or a local .wabbajack file. For local files, we'd use the file picker.
    // For now, log what would happen:
    console.log(`[harness] Would install modlist from: ${MODLIST_PATH}`);
    console.log("[harness] Waiting for install UI to appear...");

    // TODO: Wire up actual install flow when modlist gallery has Halgari's Helper
    // or implement file-picker-based install for local .wabbajack files
  });

  it("launches game and monitors for crash", async function () {
    if (!ENABLED) this.skip();
    this.timeout(LAUNCH_TIMEOUT + 30_000);

    // Navigate to mods page where launch button lives
    const navItems = await $$(".nav-item");
    if (navItems.length >= 2) {
      await navItems[1].click();
      await browser.pause(1_000);
    }

    // Look for launch button
    const buttons = await $$("button");
    let launchBtn = null;
    for (const btn of buttons) {
      const text = await btn.getText();
      if (text.includes("Launch") || text.includes("Play")) {
        launchBtn = btn;
        break;
      }
    }

    if (!launchBtn) {
      console.log("[harness] No launch button found — game may not be detected");
      return;
    }

    console.log("[harness] Launching game...");
    await launchBtn.click();

    // Wait for game to either run or crash
    await browser.pause(LAUNCH_TIMEOUT);

    // Check crash logs page for new entries
    for (const nav of navItems) {
      const text = await nav.getText();
      if (text.includes("Crash") || text.includes("Log")) {
        await nav.click();
        await browser.pause(2_000);
        break;
      }
    }

    const body = await $("body");
    const text = await body.getText();

    if (text.includes("new crash") || text.includes("crash detected")) {
      console.log("[harness] CRASH DETECTED — check crash logs");
      // Take a screenshot of the crash analysis
      await browser.saveScreenshot("screenshots/crash-detected.png");
    } else {
      console.log("[harness] No crash detected within timeout");
    }
  });

  it("runs crash-retry loop", async function () {
    if (!ENABLED) this.skip();
    this.timeout(MAX_ITERATIONS * (LAUNCH_TIMEOUT + 60_000));

    for (let i = 0; i < MAX_ITERATIONS; i++) {
      console.log(`\n[harness] === Iteration ${i + 1}/${MAX_ITERATIONS} ===`);

      // Take pre-launch screenshot
      await browser.saveScreenshot(`screenshots/harness-iter-${i + 1}-pre.png`);

      // Navigate to mods page
      const navItems = await $$(".nav-item");
      if (navItems.length >= 2) {
        await navItems[1].click();
        await browser.pause(1_000);
      }

      // Find and click launch
      const buttons = await $$("button");
      let launched = false;
      for (const btn of buttons) {
        const text = await btn.getText();
        if (text.includes("Launch") || text.includes("Play")) {
          await btn.click();
          launched = true;
          break;
        }
      }

      if (!launched) {
        console.log("[harness] No launch button — stopping loop");
        break;
      }

      // Wait for game session
      console.log(`[harness] Waiting ${LAUNCH_TIMEOUT / 1000}s for game...`);
      await browser.pause(LAUNCH_TIMEOUT);

      // Check for crash
      for (const nav of navItems) {
        const text = await nav.getText();
        if (text.includes("Crash") || text.includes("Log")) {
          await nav.click();
          await browser.pause(2_000);
          break;
        }
      }

      const body = await $("body");
      const text = await body.getText();

      await browser.saveScreenshot(`screenshots/harness-iter-${i + 1}-post.png`);

      if (text.includes("new crash") || text.includes("crash detected")) {
        console.log(`[harness] Iteration ${i + 1}: CRASH`);
        // Could toggle specific mods here before retrying
        // e.g., disable a suspect mod, swap SSEEF Wine version, etc.
      } else {
        console.log(`[harness] Iteration ${i + 1}: STABLE (no crash)`);
        break; // Game ran without crashing — success
      }
    }
  });
});
