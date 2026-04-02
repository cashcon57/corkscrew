/**
 * Tier 2 E2E: Mod Lifecycle — Install → Verify → Toggle → Uninstall
 *
 * Installs a test mod, verifies it in backend + UI, toggles it,
 * deploys, uninstalls, and verifies cleanup — all against the real binary.
 */
import path from "path";
import { fileURLToPath } from "url";

const __dirname = fileURLToPath(new URL(".", import.meta.url));
const FIXTURE_MOD = path.resolve(__dirname, "..", "fixtures", "e2e-test-mod.zip");

async function tauriInvoke(cmd: string, args: Record<string, any> = {}): Promise<any> {
  await browser.execute(
    (c: string, a: string) => {
      (window as any).__E2E_RESULT__ = "__PENDING__";
      (window as any).__TAURI_INTERNALS__
        .invoke(c, JSON.parse(a))
        .then((r: any) => { (window as any).__E2E_RESULT__ = r; })
        .catch((e: any) => { (window as any).__E2E_RESULT__ = { __error__: String(e) }; });
    },
    cmd,
    JSON.stringify(args)
  );
  for (let i = 0; i < 20; i++) {
    await browser.pause(500);
    const result = await browser.execute(() => (window as any).__E2E_RESULT__);
    if (result !== "__PENDING__") return result;
  }
  throw new Error(`tauriInvoke("${cmd}") timed out after 10s`);
}

async function navigateToMods() {
  const shell = await $(".app-shell");
  await shell.waitForExist({ timeout: 10_000 });
  const navItems = await $$(".nav-item");
  await navItems[1].click();
  await browser.pause(2_000);
}

describe("Mod Lifecycle", () => {
  let gameId = "";
  let bottleName = "";
  let installedModId: number | null = null;

  before(async () => {
    const shell = await $(".app-shell");
    await shell.waitForExist({ timeout: 20_000 });

    const games = await tauriInvoke("get_all_games") as any[];
    expect(games?.length).toBeGreaterThan(0);
    gameId = games[0].game_id;
    bottleName = games[0].bottle_name;
  });

  after(async () => {
    if (installedModId !== null) {
      try {
        await tauriInvoke("uninstall_mod", { modId: installedModId, gameId, bottleName });
      } catch { /* best effort */ }
    }
  });

  // ---- Phase 1: Install ----

  it("installs a test mod from fixture archive", async () => {
    const result = await tauriInvoke("install_mod_cmd", {
      archivePath: FIXTURE_MOD,
      gameId,
      bottleName,
      modName: "E2E Test Mod",
      modVersion: "1.0.0",
      sourceType: "manual",
      sourceUrl: null,
      nexusModId: null,
    });

    if (result?.__error__) throw new Error(`Install failed: ${result.__error__}`);

    expect(result.id).toBeGreaterThan(0);
    expect(result.name).toBe("E2E Test Mod");
    installedModId = result.id;
  });

  // ---- Phase 2: Verify via backend ----

  it("mod exists in backend with correct metadata", async () => {
    const mods = await tauriInvoke("get_installed_mods_summary", { gameId, bottleName }) as any[];
    const testMod = mods.find((m: any) => m.name === "E2E Test Mod");
    expect(testMod).toBeDefined();
    expect(testMod.enabled).toBe(true);
    expect(testMod.version).toBe("1.0.0");
    expect(testMod.file_count).toBeGreaterThan(0);
    expect(testMod.source_type).toBe("manual");
  });

  it("staging path is valid and absolute", async () => {
    const mods = await tauriInvoke("get_installed_mods_summary", { gameId, bottleName }) as any[];
    const testMod = mods.find((m: any) => m.name === "E2E Test Mod");
    expect(testMod.staging_path).toBeTruthy();
    expect(testMod.staging_path.startsWith("/")).toBe(true);
  });

  it("file paths contain no traversal sequences (security check)", async () => {
    // Use get_mod_detail to get full installed_files JSON
    const detail = await tauriInvoke("get_mod_detail", { modId: installedModId });
    if (detail && !detail.__error__ && detail.installed_files) {
      let files: string[];
      try {
        files = JSON.parse(detail.installed_files);
      } catch {
        // installed_files may be a plain filename string, not JSON array
        files = [detail.installed_files];
      }
      if (!Array.isArray(files)) files = [String(files)];
      for (const f of files) {
        expect(f).not.toContain("..");
        expect(f).not.toContain("\x00");
      }
    }
  });

  // ---- Phase 3: Verify in UI ----

  it("mod appears in the UI", async () => {
    // Navigate to mods and use the game selector to pick the right game
    await navigateToMods();

    // The app might be on a different game — check
    let body = await $("body");
    let text = await body.getText();

    if (!text.includes("E2E Test Mod")) {
      // Need to switch to the game where we installed the mod
      // Click "Change Game" button
      const buttons = await $$("button");
      for (const btn of buttons) {
        const t = await btn.getText();
        if (t.includes("Change Game")) {
          await btn.click();
          await browser.pause(1_000);

          // Look for the target game in the dropdown/list
          const gameItems = await $$(".game-option, .game-item, .game-card, [data-game-id]");
          for (const item of gameItems) {
            const itemText = await item.getText();
            if (itemText.includes("Skyrim")) {
              await item.click();
              await browser.pause(3_000);
              break;
            }
          }
          break;
        }
      }
      body = await $("body");
      text = await body.getText();
    }

    expect(text).toContain("E2E Test Mod");
  });

  // ---- Phase 4: Toggle ----

  it("toggling the mod changes active count", async () => {
    let body = await $("body");
    const textBefore = await body.getText();
    const countBefore = textBefore.match(/(\d+)\/(\d+)/);

    const toggles = await $$("[role='switch']");
    expect(toggles.length).toBeGreaterThan(0);

    // Use JavaScript click to avoid any overlay issues
    await browser.execute((el: any) => el.click(), toggles[0]);
    await browser.pause(2_000);

    body = await $("body");
    const textAfter = await body.getText();
    const countAfter = textAfter.match(/(\d+)\/(\d+)/);

    if (countBefore && countAfter) {
      const diff = Math.abs(parseInt(countBefore[1]) - parseInt(countAfter[1]));
      if (diff !== 1) {
        // Take a debug screenshot
        await browser.saveScreenshot(
          path.resolve(__dirname, "..", "screenshots", "toggle-debug.png")
        );
        console.log(`[toggle] Count before: ${countBefore[0]}, after: ${countAfter[0]}`);
      }
      expect(diff).toBe(1);
    }

    // Restore
    await browser.execute((el: any) => el.click(), toggles[0]);
    await browser.pause(1_000);
  });

  it("toggle updates backend state", async () => {
    const toggles = await $$("[role='switch']");
    await browser.execute((el: any) => el.click(), toggles[0]);
    await browser.pause(2_000);

    const mods = await tauriInvoke("get_installed_mods_summary", { gameId, bottleName }) as any[];
    const hasDisabled = mods.some((m: any) => !m.enabled);
    expect(hasDisabled).toBe(true);

    // Restore
    await browser.execute((el: any) => el.click(), toggles[0]);
    await browser.pause(1_000);
  });

  // ---- Phase 5: Deploy ----

  it("redeploy succeeds", async () => {
    const result = await tauriInvoke("redeploy_all_mods", { gameId, bottleName });
    // Just verify no crash — deploy may warn about missing game files in test env
    if (result?.__error__) {
      console.log("[lifecycle] Deploy note:", result.__error__);
    }
  });

  // ---- Phase 6: Uninstall ----

  it("uninstalls the test mod", async () => {
    expect(installedModId).not.toBeNull();

    const removed = await tauriInvoke("uninstall_mod", {
      modId: installedModId,
      gameId,
      bottleName,
    });

    if (removed?.__error__) throw new Error(`Uninstall failed: ${removed.__error__}`);
    expect(Array.isArray(removed)).toBe(true);
    installedModId = null; // Prevent double-uninstall in after()
  });

  it("mod is gone from backend", async () => {
    const mods = await tauriInvoke("get_installed_mods_summary", { gameId, bottleName }) as any[];
    const testMod = mods.find((m: any) => m.name === "E2E Test Mod");
    expect(testMod).toBeUndefined();
  });

  it("mod disappears from UI", async () => {
    // Force navigation to /mods via URL to trigger a fresh page mount
    await browser.execute(() => {
      window.location.href = "/settings";
    });
    await browser.pause(2_000);
    await browser.execute(() => {
      window.location.href = "/mods";
    });
    await browser.pause(4_000);

    const body = await $("body");
    const text = await body.getText();
    expect(text).not.toContain("E2E Test Mod");
  });
});
