import { test, expect } from './fixtures/test-fixtures';

test.describe('Dashboard (functional)', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });
    // Navigate to dashboard by clicking the sidebar brand
    await page.locator('.sidebar-brand-btn').click();
    await page.waitForSelector('.dashboard', { timeout: 10_000 });
  });

  test('shows bottle count in stat pill', async ({ page }) => {
    const statValues = page.locator('.stat-value');
    // First stat pill = bottles count, second = games count
    await expect(statValues.first()).toHaveText('2', { timeout: 5_000 });
  });

  test('shows game count in stat pill', async ({ page }) => {
    const statValues = page.locator('.stat-value');
    await expect(statValues.nth(1)).toHaveText('3', { timeout: 5_000 });
  });

  test('renders bottle cards with names', async ({ page }) => {
    const bottleCards = page.locator('.bottle-card');
    await expect(bottleCards).toHaveCount(2, { timeout: 5_000 });

    const names = page.locator('.bottle-card .card-name');
    await expect(names.first()).toHaveText('CrossOver Default');
    await expect(names.nth(1)).toHaveText('Steam Proton');
  });

  test('renders game cards with names', async ({ page }) => {
    const gameCards = page.locator('.game-card');
    await expect(gameCards).toHaveCount(3, { timeout: 5_000 });

    const names = page.locator('.game-card .card-name');
    await expect(names.first()).toHaveText('Skyrim Special Edition');
    await expect(names.nth(1)).toHaveText('Fallout 4');
    await expect(names.nth(2)).toHaveText('Hogwarts Legacy');
  });

  test('bottle cards show source badge', async ({ page }) => {
    // Each bottle card should display its source type
    const firstCard = page.locator('.bottle-card').first();
    await expect(firstCard).toContainText('CrossOver');
  });

  test('clicking game card navigates to mods page', async ({ page }) => {
    const gameCard = page.locator('.game-card').first();
    await gameCard.click();

    // Should navigate to mods page — the mods nav item becomes active
    await expect(page.locator('.nav-item').nth(1)).toHaveClass(/active/, { timeout: 5_000 });
  });
});

test.describe('Dashboard empty states', () => {
  test('shows empty bottles message when no bottles', async ({ page }) => {
    // Override get_bottles to return empty
    await page.addInitScript(`
      window.__TAURI_MOCK_OVERRIDES__ = window.__TAURI_MOCK_OVERRIDES__ || {};
      const origInvoke = window.__TAURI_INTERNALS__.invoke;
      window.__TAURI_INTERNALS__.invoke = function(cmd, args) {
        if (cmd === 'get_bottles') return Promise.resolve([]);
        return origInvoke.call(this, cmd, args);
      };
    `);
    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });
    await page.locator('.sidebar-brand-btn').click();
    await page.waitForSelector('.dashboard', { timeout: 10_000 });

    await expect(page.locator('.empty-title').first()).toHaveText('No bottles found', { timeout: 5_000 });
  });

  test('shows empty games message when no games', async ({ page }) => {
    await page.addInitScript(`
      const origInvoke = window.__TAURI_INTERNALS__.invoke;
      window.__TAURI_INTERNALS__.invoke = function(cmd, args) {
        if (cmd === 'get_all_games') return Promise.resolve([]);
        return origInvoke.call(this, cmd, args);
      };
    `);
    await page.goto('/');
    await page.waitForSelector('.dashboard', { timeout: 15_000 });

    await expect(page.locator('.empty-title').last()).toHaveText('No games detected', { timeout: 5_000 });
  });
});
