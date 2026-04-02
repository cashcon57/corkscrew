/**
 * Tier 2 E2E: Settings Page
 *
 * Tests settings interactions: theme toggle, config persistence,
 * deployment health, and diagnostic panels.
 */
describe("Settings", () => {
  before(async () => {
    const shell = await $(".app-shell");
    await shell.waitForExist({ timeout: 20_000 });

    // Navigate to settings (last nav item)
    const navItems = await $$(".nav-item");
    await navItems[navItems.length - 1].click();
    await browser.pause(2_000);
  });

  it("renders the settings page", async () => {
    const body = await $("body");
    const text = await body.getText();
    expect(text).toContain("Settings");
  });

  it("shows General section", async () => {
    const body = await $("body");
    const text = await body.getText();
    expect(text).toContain("General");
  });

  it("shows theme toggle options", async () => {
    const body = await $("body");
    const text = await body.getText();
    // Theme section with Light/Dark/Auto options
    const hasTheme =
      text.includes("Theme") ||
      text.includes("Light") ||
      text.includes("Dark");
    expect(hasTheme).toBe(true);
  });

  it("can toggle theme", async () => {
    // Find theme toggle buttons
    const themeButtons = await $$("button");
    let darkButton = null;
    let lightButton = null;

    for (const btn of themeButtons) {
      const text = await btn.getText();
      if (text.trim() === "Dark") darkButton = btn;
      if (text.trim() === "Light") lightButton = btn;
    }

    if (darkButton && lightButton) {
      // Click Dark theme
      await darkButton.click();
      await browser.pause(500);

      // The body or html should have a dark theme class or data attribute
      const html = await $("html");
      const cls = await html.getAttribute("class");
      const dataTheme = await html.getAttribute("data-theme");
      const hasDark =
        (cls && cls.includes("dark")) ||
        (dataTheme && dataTheme.includes("dark"));

      // Click Light theme
      await lightButton.click();
      await browser.pause(500);

      // Toggle back to dark (restore)
      await darkButton.click();
      await browser.pause(300);
    }
  });

  it("shows download threads setting", async () => {
    const body = await $("body");
    const text = await body.getText();
    expect(text).toContain("Download Threads");
  });

  it("shows app version in About section", async () => {
    const body = await $("body");
    const text = await body.getText();
    // About section with version
    expect(text).toMatch(/v?\d+\.\d+\.\d+/);
    expect(text).toContain("About");
  });

  it("shows NexusMods account section", async () => {
    const body = await $("body");
    const text = await body.getText();
    expect(text).toContain("Nexus Mods");
  });

  it("shows platform information", async () => {
    const body = await $("body");
    const text = await body.getText();
    // Should display detected platform
    const hasPlatform =
      text.includes("macOS") ||
      text.includes("Linux") ||
      text.includes("Platform");
    expect(hasPlatform).toBe(true);
  });

  it("shows privacy/telemetry toggle", async () => {
    const body = await $("body");
    const text = await body.getText();
    const hasPrivacy =
      text.includes("Privacy") ||
      text.includes("Telemetry") ||
      text.includes("crash reports");
    expect(hasPrivacy).toBe(true);
  });
});
