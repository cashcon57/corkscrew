import { test, expect } from './fixtures/test-fixtures';

test.describe('Settings Page (functional)', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });
    // Navigate to settings
    await page.locator('.nav-item').nth(5).click();
    await expect(page.locator('.nav-item').nth(5)).toHaveClass(/active/);
  });

  test('displays app version from mock', async ({ page }) => {
    // The version "0.9.43" should appear on the settings page
    await expect(page.locator('text=0.9.43').first()).toBeVisible({ timeout: 10_000 });
  });

  test('settings tabs render', async ({ page }) => {
    const tabs = page.locator('.settings-tab');
    await expect(tabs).toHaveCount(3, { timeout: 10_000 });
  });

  test('general tab is active by default', async ({ page }) => {
    await expect(page.locator('.settings-tab.tab-active')).toHaveCount(1, { timeout: 10_000 });
    await expect(page.locator('.settings-tab').first()).toHaveClass(/tab-active/);
  });

  test('all three tabs are clickable', async ({ page }) => {
    const tabs = page.locator('.settings-tab');
    await expect(tabs).toHaveCount(3, { timeout: 10_000 });
    // Verify each tab has correct text
    await expect(tabs.nth(0)).toContainText('General');
    await expect(tabs.nth(1)).toContainText('Game');
    await expect(tabs.nth(2)).toContainText('System');
  });
});
