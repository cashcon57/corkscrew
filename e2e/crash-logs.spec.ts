import { test, expect } from './fixtures/test-fixtures';

test.describe('Crash Logs Page (functional)', () => {
  test('renders empty state without runtime errors', async ({ page }) => {
    const pageErrors: string[] = [];
    page.on('pageerror', (err) => pageErrors.push(err.message));

    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });

    const navItem = page.locator('.nav-item').nth(4);
    await navItem.click();
    await expect(navItem).toHaveClass(/active/);

    await expect(page.locator('text=No crash logs found')).toBeVisible({ timeout: 10_000 });
    expect(pageErrors).toEqual([]);
  });

  test.describe('when the backend returns null', () => {
    test.use({ mockOverrides: { find_crash_logs_cmd: null } });

    test('falls back to empty state', async ({ page }) => {
      const pageErrors: string[] = [];
      page.on('pageerror', (err) => pageErrors.push(err.message));

      await page.goto('/');
      await page.waitForSelector('.app-shell', { timeout: 15_000 });
      await page.locator('.nav-item').nth(4).click();

      await expect(page.locator('text=No crash logs found')).toBeVisible({ timeout: 10_000 });
      expect(pageErrors).toEqual([]);
    });
  });
});
