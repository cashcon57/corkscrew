import { test, expect } from '@playwright/test';

test.describe('Navigation', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });
  });

  const navPages = [
    { label: 'Discover', index: 0 },
    { label: 'Mods', index: 1 },
    { label: 'Load Order', index: 2 },
    { label: 'Profiles', index: 3 },
    { label: 'Crash Logs', index: 4 },
    { label: 'Settings', index: 5 },
  ];

  test('Mods nav-item is active by default', async ({ page }) => {
    const activeItems = page.locator('.nav-item.active');
    await expect(activeItems).toHaveCount(1);
    await expect(page.locator('.nav-item').nth(1)).toHaveClass(/active/);
  });

  for (const nav of navPages) {
    test(`clicking "${nav.label}" sets active state`, async ({ page }) => {
      const navItem = page.locator('.nav-item').nth(nav.index);
      await navItem.click();
      await expect(navItem).toHaveClass(/active/, { timeout: 5_000 });
    });
  }

  test('only one nav item is active at a time', async ({ page }) => {
    await page.locator('.nav-item').nth(1).click();
    await expect(page.locator('.nav-item.active')).toHaveCount(1);

    await page.locator('.nav-item').nth(5).click();
    await expect(page.locator('.nav-item.active')).toHaveCount(1);
    await expect(page.locator('.nav-item').nth(5)).toHaveClass(/active/);
    await expect(page.locator('.nav-item').nth(1)).not.toHaveClass(/active/);
  });

  test('clicking brand button returns to dashboard', async ({ page }) => {
    await page.locator('.nav-item').nth(5).click();
    await expect(page.locator('.nav-item').nth(5)).toHaveClass(/active/);

    await page.locator('.sidebar-brand-btn').click();
    // No nav item should be active when on dashboard
    await expect(page.locator('.nav-item.active')).toHaveCount(0);
  });

  test('URL stays at / after navigation', async ({ page }) => {
    await page.locator('.nav-item').nth(1).click();
    expect(page.url()).toMatch(/\/$/);
  });
});
