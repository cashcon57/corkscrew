import { test, expect } from './fixtures/test-fixtures';

async function openProfiles(page: import('@playwright/test').Page) {
  await page.goto('/');
  await page.waitForSelector('.app-shell', { timeout: 15_000 });
  await page.locator('.nav-item').nth(3).click();
  await expect(page.locator('.nav-item').nth(3)).toHaveClass(/active/);
}

test.describe('Profiles Page (functional)', () => {
  test('renders profiles returned by list_profiles_cmd', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (e) => errors.push(e.message));

    await openProfiles(page);

    await expect(page.locator('.profile-card')).toHaveCount(1, { timeout: 10_000 });
    await expect(page.locator('.profile-name').first()).toContainText('default');
    expect(errors).toEqual([]);
  });

  test.describe('when list_profiles_cmd returns null', () => {
    test.use({ mockOverrides: { list_profiles_cmd: null } });

    test('shows empty state instead of crashing', async ({ page }) => {
      const errors: string[] = [];
      page.on('pageerror', (e) => errors.push(e.message));

      await openProfiles(page);

      await expect(page.locator('.empty-title')).toHaveText('No profiles yet', { timeout: 10_000 });
      expect(errors.filter((m) => m.includes("reading 'length'"))).toEqual([]);
    });
  });
});
