/**
 * Tier 2 E2E: App Launch
 *
 * Tests that the compiled Corkscrew binary starts, renders the shell,
 * and reaches a usable state via the real Tauri webview + Rust backend.
 */
describe("App Launch", () => {
  it("renders the app shell", async () => {
    const shell = await $(".app-shell");
    await shell.waitForExist({ timeout: 20_000 });
    expect(await shell.isDisplayed()).toBe(true);
  });

  it("shows the sidebar navigation", async () => {
    const sidebar = await $(".sidebar");
    await sidebar.waitForExist({ timeout: 10_000 });
    expect(await sidebar.isDisplayed()).toBe(true);
  });

  it("displays the app version in sidebar", async () => {
    const sidebar = await $(".sidebar");
    await sidebar.waitForExist({ timeout: 10_000 });
    const text = await sidebar.getText();
    // Should contain semver pattern like "v0.9.58"
    expect(text).toMatch(/v?\d+\.\d+\.\d+/);
  });

  it("renders navigation items", async () => {
    const navItems = await $$(".nav-item");
    // Should have at least Dashboard, Mods, Collections, Settings
    expect(navItems.length).toBeGreaterThanOrEqual(4);
  });
});
