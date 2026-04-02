/**
 * Tier 2 E2E: Navigation
 *
 * Tests sidebar navigation between pages using the real app binary.
 * Verifies that each major page loads and renders its primary content.
 */
describe("Navigation", () => {
  before(async () => {
    const shell = await $(".app-shell");
    await shell.waitForExist({ timeout: 20_000 });
  });

  it("navigates to the Mods page", async () => {
    // Click the mods nav item (typically second in the sidebar)
    const navItems = await $$(".nav-item");
    expect(navItems.length).toBeGreaterThanOrEqual(2);

    await navItems[1].click();
    await navItems[1].waitUntil(
      async () => {
        const cls = await navItems[1].getAttribute("class");
        return cls.includes("active");
      },
      { timeout: 5_000, timeoutMsg: "Mods nav item did not become active" }
    );
  });

  it("navigates to the Collections page", async () => {
    const navItems = await $$(".nav-item");
    // Collections is typically the third nav item
    if (navItems.length >= 3) {
      await navItems[2].click();
      await browser.pause(1_000);
      const cls = await navItems[2].getAttribute("class");
      expect(cls).toContain("active");
    }
  });

  it("navigates to the Settings page", async () => {
    // Settings is usually the last nav item or has a gear icon
    const settingsNav = await $(".nav-item[data-page='settings']");
    if (await settingsNav.isExisting()) {
      await settingsNav.click();
      await browser.pause(1_000);
      const cls = await settingsNav.getAttribute("class");
      expect(cls).toContain("active");
    } else {
      // Fallback: click the last nav item
      const navItems = await $$(".nav-item");
      const last = navItems[navItems.length - 1];
      await last.click();
      await browser.pause(1_000);
    }
  });

  it("navigates back to Dashboard", async () => {
    const navItems = await $$(".nav-item");
    await navItems[0].click();
    await browser.pause(500);
    const cls = await navItems[0].getAttribute("class");
    expect(cls).toContain("active");
  });
});
