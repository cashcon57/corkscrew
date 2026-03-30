import { test, expect } from './fixtures/test-fixtures';

test.describe('Mods Page (functional)', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });
    // Navigate to mods page
    await page.locator('.nav-item').nth(1).click();
    await expect(page.locator('.nav-item').nth(1)).toHaveClass(/active/);
  });

  test('renders mod list with enabled mods visible', async ({ page }) => {
    // Wait for the toggle switches to appear (indicates mods loaded)
    const toggles = page.locator('.toggle-switch');
    await expect(toggles.first()).toBeVisible({ timeout: 10_000 });
    // 4 enabled mods should be visible (disabled are collapsed)
    const count = await toggles.count();
    expect(count).toBeGreaterThanOrEqual(4);
  });

  test('shows correct game header', async ({ page }) => {
    // The page header should show the game name
    await expect(page.locator('text=Skyrim Special Edition').first()).toBeVisible({ timeout: 10_000 });
  });

  test('shows mod count in header', async ({ page }) => {
    // "4/6 mods active" or similar count text (Skyrim SE only — mock filters by gameId)
    await expect(page.locator('text=4/6').first()).toBeVisible({ timeout: 10_000 });
  });

  test('shows version text for mods', async ({ page }) => {
    await expect(page.locator('.toggle-switch').first()).toBeVisible({ timeout: 10_000 });
    // Version "5.2SE" (SkyUI) should appear
    await expect(page.locator('.version-text', { hasText: '5.2SE' })).toBeVisible();
    // Version "4.3.2" (USSEP) should appear
    await expect(page.locator('.version-text', { hasText: '4.3.2' })).toBeVisible();
  });

  test('enabled mods have toggle-on class', async ({ page }) => {
    await expect(page.locator('.toggle-switch').first()).toBeVisible({ timeout: 10_000 });
    // Mock has 4 enabled Skyrim mods (filtered by gameId)
    const togglesOn = page.locator('.toggle-switch.toggle-on');
    await expect(togglesOn).toHaveCount(4);
  });

  test('disabled mods section shows count', async ({ page }) => {
    await expect(page.locator('.toggle-switch').first()).toBeVisible({ timeout: 10_000 });
    // Disabled separator has label "Disabled" and group-count "2"
    await expect(page.locator('.disabled-separator-label')).toHaveText('Disabled', { timeout: 5_000 });
    await expect(page.locator('.disabled-separator .group-count')).toHaveText('2');
  });

  test('mod categories are displayed', async ({ page }) => {
    await expect(page.locator('.toggle-switch').first()).toBeVisible({ timeout: 10_000 });
    // Categories from mock data
    await expect(page.locator('.category-label', { hasText: 'UI' })).toBeVisible();
    await expect(page.locator('.category-label', { hasText: 'Bug Fixes' })).toBeVisible();
    await expect(page.locator('.category-label', { hasText: 'Gameplay' })).toBeVisible();
  });

  test('mod source labels are displayed', async ({ page }) => {
    await expect(page.locator('.toggle-switch').first()).toBeVisible({ timeout: 10_000 });
    // Most mods are from Nexus
    await expect(page.locator('.origin-nexus').first()).toBeVisible();
  });

  test('action buttons are visible', async ({ page }) => {
    await expect(page.locator('.toggle-switch').first()).toBeVisible({ timeout: 10_000 });
    // Install Mod, Deploy buttons should be present
    await expect(page.locator('text=Install Mod').first()).toBeVisible();
    await expect(page.locator('text=Deploy').first()).toBeVisible();
  });

  test('deployment status panel shows', async ({ page }) => {
    await expect(page.locator('.toggle-switch').first()).toBeVisible({ timeout: 10_000 });
    // Deployment section should be visible
    await expect(page.locator('text=DEPLOYMENT').first()).toBeVisible();
  });
});
