import { test, expect } from '@playwright/test';

// navPageIds order: ["mods", "plugins", "discover", "profiles", "logs", "settings"]
// Nav item display order: [Discover(0), Mods(1), Load Order(2), Profiles(3), Crash Logs(4), Settings(5)]
// So Cmd+1 = mods = nav index 1, Cmd+2 = plugins = nav index 2, Cmd+3 = discover = nav index 0, etc.
const shortcutToNavIndex: Record<string, number> = {
  '1': 1,  // mods
  '2': 2,  // plugins
  '3': 0,  // discover
  '4': 3,  // profiles
  '5': 4,  // logs
  '6': 5,  // settings
};

test.describe('Keyboard Shortcuts', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });
  });

  const modifier = process.platform === 'darwin' ? 'Meta' : 'Control';

  test('Cmd/Ctrl+B toggles sidebar collapse', async ({ page }) => {
    await expect(page.locator('nav.sidebar')).not.toHaveClass(/collapsed/);

    await page.keyboard.press(`${modifier}+b`);
    await expect(page.locator('nav.sidebar')).toHaveClass(/collapsed/);

    await page.keyboard.press(`${modifier}+b`);
    await expect(page.locator('nav.sidebar')).not.toHaveClass(/collapsed/);
  });

  for (const [key, navIndex] of Object.entries(shortcutToNavIndex)) {
    test(`Cmd/Ctrl+${key} navigates to correct page`, async ({ page }) => {
      await page.keyboard.press(`${modifier}+${key}`);
      await expect(page.locator('.nav-item').nth(navIndex)).toHaveClass(/active/, { timeout: 3_000 });
    });
  }

  test('Cmd/Ctrl+K opens spotlight search', async ({ page }) => {
    await page.keyboard.press(`${modifier}+k`);
    await expect(page.locator('.spotlight-overlay')).toBeVisible({ timeout: 3_000 });
  });

  test('Escape closes spotlight search', async ({ page }) => {
    await page.keyboard.press(`${modifier}+k`);
    await expect(page.locator('.spotlight-overlay')).toBeVisible({ timeout: 3_000 });

    await page.keyboard.press('Escape');
    await expect(page.locator('.spotlight-overlay')).not.toBeVisible();
  });
});
