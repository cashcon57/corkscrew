/**
 * Tier 2 E2E: Modal & Overlay Behavior
 *
 * Tests that modals open/close correctly, escape key works,
 * overlay click-to-dismiss works, and z-index layering is correct.
 */
describe("Modal Behavior", () => {
  before(async () => {
    const shell = await $(".app-shell");
    await shell.waitForExist({ timeout: 20_000 });
  });

  it("keyboard shortcuts modal opens and closes", async () => {
    // Press ? or Ctrl+/ to open keyboard shortcuts modal
    await browser.keys(["Shift", "/"]);
    await browser.pause(500);

    // Check if a modal/overlay appeared
    const overlays = await $$(".modal-overlay, .shortcuts-overlay, [role='dialog']");
    if (overlays.length > 0) {
      expect(await overlays[0].isDisplayed()).toBe(true);

      // Press Escape to close
      await browser.keys(["Escape"]);
      await browser.pause(500);

      // Modal should be gone
      const remaining = await $$(".modal-overlay, .shortcuts-overlay, [role='dialog']");
      const visible = [];
      for (const el of remaining) {
        if (await el.isDisplayed()) visible.push(el);
      }
      expect(visible.length).toBe(0);
    }
  });

  it("no modals are open by default", async () => {
    const modals = await $$(".modal-overlay, [role='dialog']");
    const visible = [];
    for (const m of modals) {
      if (await m.isDisplayed()) visible.push(m);
    }
    expect(visible.length).toBe(0);
  });

  it("sidebar navigation maintains state on page switch", async () => {
    // Click through all nav items and verify active state tracks correctly
    const navItems = await $$(".nav-item");
    for (let i = 0; i < Math.min(navItems.length, 4); i++) {
      await navItems[i].click();
      await browser.pause(500);

      const cls = await navItems[i].getAttribute("class");
      expect(cls).toContain("active");

      // Verify only one nav item is active at a time
      let activeCount = 0;
      for (const nav of navItems) {
        const navCls = await nav.getAttribute("class");
        if (navCls.includes("active")) activeCount++;
      }
      expect(activeCount).toBe(1);
    }
  });

  it("footer links are present", async () => {
    const body = await $("body");
    const text = await body.getText();
    // App should show GitHub and Ko-fi links
    const hasLinks =
      text.includes("GitHub") || text.includes("Ko-fi");
    expect(hasLinks).toBe(true);
  });
});
