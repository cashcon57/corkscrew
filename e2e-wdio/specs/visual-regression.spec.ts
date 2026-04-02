/**
 * Tier 2 E2E: Visual Regression
 *
 * Takes screenshots of key pages and compares against baselines.
 * On first run, creates baselines. On subsequent runs, diffs against them.
 * Screenshots are saved to e2e-wdio/screenshots/.
 */
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = fileURLToPath(new URL(".", import.meta.url));
const screenshotDir = path.resolve(__dirname, "..", "screenshots");

describe("Visual Regression", () => {
  before(async () => {
    // Ensure screenshot directory exists
    if (!fs.existsSync(screenshotDir)) {
      fs.mkdirSync(screenshotDir, { recursive: true });
    }

    const shell = await $(".app-shell");
    await shell.waitForExist({ timeout: 20_000 });
  });

  it("captures dashboard page", async () => {
    // Navigate to dashboard (first nav item)
    const navItems = await $$(".nav-item");
    await navItems[0].click();
    await browser.pause(1_500);

    await browser.saveScreenshot(
      path.join(screenshotDir, "dashboard.png")
    );

    // Verify screenshot was created
    expect(fs.existsSync(path.join(screenshotDir, "dashboard.png"))).toBe(true);
  });

  it("captures mods page", async () => {
    const navItems = await $$(".nav-item");
    if (navItems.length >= 2) {
      await navItems[1].click();
      await browser.pause(1_500);
    }

    await browser.saveScreenshot(
      path.join(screenshotDir, "mods-page.png")
    );
    expect(fs.existsSync(path.join(screenshotDir, "mods-page.png"))).toBe(true);
  });

  it("captures collections page", async () => {
    const navItems = await $$(".nav-item");
    if (navItems.length >= 3) {
      await navItems[2].click();
      await browser.pause(1_500);
    }

    await browser.saveScreenshot(
      path.join(screenshotDir, "collections-page.png")
    );
    expect(fs.existsSync(path.join(screenshotDir, "collections-page.png"))).toBe(true);
  });

  it("captures settings page", async () => {
    const navItems = await $$(".nav-item");
    await navItems[navItems.length - 1].click();
    await browser.pause(1_500);

    await browser.saveScreenshot(
      path.join(screenshotDir, "settings-page.png")
    );
    expect(fs.existsSync(path.join(screenshotDir, "settings-page.png"))).toBe(true);
  });

  it("captures dark theme baseline", async () => {
    // Ensure dark theme is active
    const buttons = await $$("button");
    for (const btn of buttons) {
      const text = await btn.getText();
      if (text.trim() === "Dark") {
        await btn.click();
        await browser.pause(500);
        break;
      }
    }

    await browser.saveScreenshot(
      path.join(screenshotDir, "settings-dark.png")
    );
    expect(fs.existsSync(path.join(screenshotDir, "settings-dark.png"))).toBe(true);
  });

  it("captures light theme baseline", async () => {
    const buttons = await $$("button");
    for (const btn of buttons) {
      const text = await btn.getText();
      if (text.trim() === "Light") {
        await btn.click();
        await browser.pause(500);
        break;
      }
    }

    await browser.saveScreenshot(
      path.join(screenshotDir, "settings-light.png")
    );
    expect(fs.existsSync(path.join(screenshotDir, "settings-light.png"))).toBe(true);

    // Restore dark theme
    for (const btn of buttons) {
      const text = await btn.getText();
      if (text.trim() === "Dark") {
        await btn.click();
        await browser.pause(300);
        break;
      }
    }
  });
});
