import { test, expect } from '@playwright/test';

test.describe('App Shell Layout', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });
  });

  test('renders the app-shell container', async ({ page }) => {
    await expect(page.locator('.app-shell')).toBeVisible();
  });

  test('renders the sidebar', async ({ page }) => {
    await expect(page.locator('nav.sidebar')).toBeVisible();
  });

  test('renders the content column', async ({ page }) => {
    await expect(page.locator('.content-column')).toBeVisible();
  });

  test('renders the top bar', async ({ page }) => {
    await expect(page.locator('.top-bar')).toBeVisible();
  });

  test('renders the main content area', async ({ page }) => {
    await expect(page.locator('main.content')).toBeVisible();
  });

  test('sidebar contains brand section with Corkscrew name', async ({ page }) => {
    await expect(page.locator('.sidebar-brand-section')).toBeVisible();
    await expect(page.locator('.brand-name')).toHaveText('Corkscrew');
    await expect(page.locator('.brand-tagline')).toHaveText('Wine Dashboard');
  });

  test('sidebar contains exactly 6 nav items', async ({ page }) => {
    const navItems = page.locator('.nav-item');
    await expect(navItems).toHaveCount(6);
  });

  test('nav items have correct labels', async ({ page }) => {
    const labels = page.locator('.nav-label');
    await expect(labels).toHaveCount(6);
    const texts = await labels.allTextContents();
    expect(texts).toEqual(['Discover', 'Mods', 'Load Order', 'Profiles', 'Crash Logs', 'Settings']);
  });

  test('sidebar footer is visible with collapse button', async ({ page }) => {
    await expect(page.locator('.sidebar-footer')).toBeVisible();
    await expect(page.locator('.sidebar-collapse-btn')).toBeVisible();
  });

  test('chat toggle button exists in sidebar footer', async ({ page }) => {
    await expect(page.locator('.chat-toggle-btn')).toBeVisible();
  });
});
