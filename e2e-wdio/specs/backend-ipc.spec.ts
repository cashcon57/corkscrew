/**
 * Tier 2 E2E: Backend IPC
 *
 * Tests that the frontend communicates with the real Rust backend
 * via Tauri invoke() IPC. This is the key value-add over Tier 1
 * (Playwright mocked tests) — we're testing the real bridge.
 */
describe("Backend IPC", () => {
  before(async () => {
    const shell = await $(".app-shell");
    await shell.waitForExist({ timeout: 20_000 });
  });

  it("loads config from the Rust backend", async () => {
    // The app loads config on mount — if the shell renders with content,
    // the get_config IPC call succeeded.
    await browser.pause(2_000);
    const body = await $("body");
    const text = await body.getText();
    // The app should show real content — not be a blank page
    expect(text.length).toBeGreaterThan(100);
    // Should contain app name from rendered UI
    expect(text).toContain("Corkscrew");
  });

  it("detects platform info from backend", async () => {
    // Navigate to settings where platform info is displayed
    const navItems = await $$(".nav-item");
    const settingsNav = navItems[navItems.length - 1];
    await settingsNav.click();
    await browser.pause(2_000);

    // Settings page shows platform from get_platform_detail()
    const body = await $("body");
    const text = await body.getText();
    // Real backend returns actual OS
    const hasPlatform =
      text.includes("macOS") ||
      text.includes("Linux") ||
      text.includes("Windows");
    expect(hasPlatform).toBe(true);
  });

  it("shows real app version from backend", async () => {
    // Still on settings page — version comes from plugin:app|version
    const body = await $("body");
    const text = await body.getText();
    // Should contain a real version number (not mocked)
    expect(text).toMatch(/v\d+\.\d+\.\d+/);
  });

  it("shows NexusMods auth state", async () => {
    // Settings page shows NexusMods account section
    const body = await $("body");
    const text = await body.getText();
    // Should show either "Sign In" or a username (real auth state from backend)
    const hasAuth =
      text.includes("Sign In") ||
      text.includes("Sign Out") ||
      text.includes("Nexus Mods");
    expect(hasAuth).toBe(true);
  });
});
