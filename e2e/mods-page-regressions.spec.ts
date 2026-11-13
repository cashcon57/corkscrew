import { test, expect } from './fixtures/test-fixtures';
import type { Page } from '@playwright/test';

/**
 * Regression tests for Mods page bugs from the v0.14.12 external bug report:
 *   #3  — reload effects looped forever after a profile switch (wedged UI,
 *         "Loading disk info..." never finished)
 *   #16 — mod list collapsed to 0px at narrow window widths
 *   #17 — raw "\u2014" escape rendered in the Category column
 */

const NAV_MODS = 1;
const NAV_PROFILES = 3;

async function gotoMods(page: Page) {
  await page.goto('/');
  await page.waitForSelector('.app-shell', { timeout: 15_000 });
  await page.locator('.nav-item').nth(NAV_MODS).click();
  await expect(page.locator('.nav-item').nth(NAV_MODS)).toHaveClass(/active/);
  await expect(page.locator('.toggle-switch').first()).toBeVisible({ timeout: 10_000 });
}

function countCalls(page: Page, cmd: string): Promise<number> {
  return page.evaluate(
    (c) => ((window as unknown as { __TAURI_MOCK_LOG__?: { cmd: string }[] }).__TAURI_MOCK_LOG__ ?? [])
      .filter((e) => e.cmd === c).length,
    cmd,
  );
}

test.describe('Mods page: reload effects (#3)', () => {
  test.use({
    mockOverrides: {
      list_profiles_cmd: [
        { id: 1, game_id: 'skyrimse', bottle_name: 'CrossOver Default', name: 'Default', is_active: true, created_at: '2026-03-01T12:00:00Z' },
        { id: 2, game_id: 'skyrimse', bottle_name: 'CrossOver Default', name: 'Alt', is_active: false, created_at: '2026-03-02T12:00:00Z' },
      ],
    },
  });

  test('activating a profile reloads the mod list a bounded number of times', async ({ page }) => {
    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });

    // Activate a profile — this bumps modStateVersion.
    await page.locator('.nav-item').nth(NAV_PROFILES).click();
    const activate = page.getByRole('button', { name: 'Activate', exact: true }).first();
    await expect(activate).toBeVisible({ timeout: 10_000 });
    await activate.click();
    await expect.poll(() => countCalls(page, 'activate_profile')).toBe(1);
    await expect(page.locator('text=Profile activated').first()).toBeVisible({ timeout: 5_000 });

    // Open the Mods page; its modStateVersion effect fires on mount.
    const before = await countCalls(page, 'get_installed_mods_summary');
    await page.locator('.nav-item').nth(NAV_MODS).click();
    await expect(page.locator('.toggle-switch').first()).toBeVisible({ timeout: 10_000 });

    // The mock IPC resolves instantly, so a feedback loop would rack up
    // hundreds of calls in this window. A healthy page makes a handful
    // (game-change load + post-state-change load).
    await page.waitForTimeout(1_500);
    const loads = (await countCalls(page, 'get_installed_mods_summary')) - before;
    expect(loads).toBeGreaterThan(0);
    expect(loads).toBeLessThanOrEqual(4);

    // And it has settled: no further loads once idle.
    const settled = await countCalls(page, 'get_installed_mods_summary');
    await page.waitForTimeout(1_000);
    expect(await countCalls(page, 'get_installed_mods_summary')).toBe(settled);

    // The disk widget isn't starved behind the reload storm.
    await expect(page.locator('.budget-loading-text')).toHaveCount(0, { timeout: 5_000 });
  });
});

test.describe('Mods page: narrow widths (#16)', () => {
  // Make the ActionQueue render (it was the flex sibling that crushed the list).
  test.use({
    mockOverrides: {
      get_deployment_stats: {
        total_files: 172, deployed_files: 172, missing_files: 0, orphaned_files: 0,
        total_mods: 6, is_deployed: false, needs_redeploy: true,
      },
    },
  });

  for (const width of [800, 1000, 1300]) {
    test(`mod list is visible at ${width}px`, async ({ page }) => {
      await page.setViewportSize({ width, height: 800 });
      await gotoMods(page);
      await expect(page.locator('.action-queue')).toBeVisible();

      const body = await page.locator('.table-body').boundingBox();
      expect(body).not.toBeNull();
      expect(body!.width).toBeGreaterThan(200);

      // Name, Version and Actions columns of the first row get real width.
      const row = page.locator('.table-body .table-row').first();
      for (const col of ['.col-name', '.col-version', '.col-actions']) {
        const box = await row.locator(col).boundingBox();
        expect(box, `${col} at ${width}px`).not.toBeNull();
        expect(box!.width, `${col} width at ${width}px`).toBeGreaterThan(20);
      }

      // The ActionQueue sits above the table, not beside it.
      const queue = await page.locator('.action-queue').boundingBox();
      const table = await page.locator('.mod-table-container').boundingBox();
      expect(queue!.y + queue!.height).toBeLessThanOrEqual(table!.y + 1);
    });
  }
});

test.describe('Mods page: category placeholder (#17)', () => {
  test.use({
    mockOverrides: {
      get_installed_mods_summary: [
        { id: 1, game_id: 'skyrimse', bottle_name: 'CrossOver Default', nexus_mod_id: 12604, nexus_file_id: null, source_url: null, source_type: 'nexus', name: 'Uncategorized Mod', version: '1.0', archive_name: 'x.7z', file_count: 1, installed_at: '2026-03-01T12:00:00Z', enabled: true, staging_path: '/tmp/staging/1', install_priority: 1, collection_name: null, user_notes: null, user_tags: [], auto_category: null, collection_optional: false },
      ],
    },
  });

  test('renders an em dash, not a raw escape, for a missing category', async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 800 });
    await gotoMods(page);
    const cell = page.locator('.table-body .table-row .col-category').first();
    await expect(cell).toHaveText('—');
    await expect(cell).not.toContainText('\\u');
  });
});

test.describe('Mods page: malformed stored update results', () => {
  // The backend always returns an array, but the page must not crash if the
  // IPC layer ever hands back null (the e2e mock used to do exactly that).
  test.use({ mockOverrides: { get_mod_update_results: null } });

  test('renders the mod list without uncaught errors when update results are null', async ({ page }) => {
    const pageErrors: string[] = [];
    page.on('pageerror', (err) => pageErrors.push(err.message));

    await gotoMods(page);
    await expect.poll(() => countCalls(page, 'get_mod_update_results')).toBeGreaterThan(0);
    await page.waitForTimeout(500);

    expect(pageErrors.filter((m) => m.includes('modUpdates'))).toEqual([]);
    await expect(page.locator('.toggle-switch').first()).toBeVisible();
  });
});
