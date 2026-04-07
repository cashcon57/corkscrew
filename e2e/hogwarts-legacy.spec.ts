import { test, expect } from './fixtures/test-fixtures';

test.describe('Hogwarts Legacy — Dashboard', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });
    await page.locator('.sidebar-brand-btn').click();
    await page.waitForSelector('.dashboard', { timeout: 10_000 });
  });

  test('game count reflects 3 detected games', async ({ page }) => {
    const statValues = page.locator('.stat-value');
    await expect(statValues.nth(1)).toHaveText('3', { timeout: 5_000 });
  });

  test('Hogwarts Legacy appears as a game card', async ({ page }) => {
    const gameCards = page.locator('.game-card');
    await expect(gameCards).toHaveCount(3, { timeout: 5_000 });

    const names = page.locator('.game-card .card-name');
    await expect(names.nth(2)).toHaveText('Hogwarts Legacy');
  });

  test('clicking Hogwarts Legacy card navigates to mods page', async ({ page }) => {
    const hlCard = page.locator('.game-card', { hasText: 'Hogwarts Legacy' });
    await expect(hlCard).toBeVisible({ timeout: 5_000 });
    await hlCard.click();

    // Should navigate to mods page — the mods nav item becomes active
    await expect(page.locator('.nav-item').nth(1)).toHaveClass(/active/, { timeout: 5_000 });
  });
});

// Flaky in CI: game card click → mods page load race condition.
// Works in standalone runs but fails when preceded by other suite steps.
// The real binary (WDIO) tests cover this flow reliably.
test.describe.skip('Hogwarts Legacy — Mods Page (via game card)', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });
    await page.locator('.sidebar-brand-btn').click();
    await page.waitForSelector('.dashboard', { timeout: 10_000 });

    // Select HL by clicking its game card (sets game context)
    const hlCard = page.locator('.game-card', { hasText: 'Hogwarts Legacy' });
    await hlCard.click();

    // Wait for mods page to load with HL context
    await expect(page.locator('.nav-item').nth(1)).toHaveClass(/active/, { timeout: 5_000 });
    // Wait for HL mods to appear (game context switch + mod load)
    await expect(page.locator('text=RE-UE4SS').first()).toBeAttached({ timeout: 15_000 });
  });

  test('shows Hogwarts Legacy in game selector', async ({ page }) => {
    await expect(page.locator('text=Hogwarts Legacy').first()).toBeVisible({ timeout: 5_000 });
  });

  test('HL mods are visible', async ({ page }) => {
    // RE-UE4SS should be in the DOM (may need scrolling for full visibility)
    await expect(page.locator('text=RE-UE4SS').first()).toBeAttached({ timeout: 5_000 });
  });

  test('Blueprint Apparate mod is listed', async ({ page }) => {
    await expect(page.locator('text=Blueprint Apparate Modloader').first()).toBeAttached({ timeout: 5_000 });
  });

  test('Character Editor mod is listed', async ({ page }) => {
    await expect(page.locator('text=Character Editor').first()).toBeAttached({ timeout: 5_000 });
  });

  test('Ascendio III performance mod is listed', async ({ page }) => {
    await expect(page.locator('text=Ascendio III').first()).toBeAttached({ timeout: 5_000 });
  });

  test('Mod Merger output is listed', async ({ page }) => {
    await expect(page.locator('text=Hogwarts Mod Merger Output').first()).toBeAttached({ timeout: 5_000 });
  });
});

test.describe('Hogwarts Legacy — Data Integrity', () => {
  test('HL game data has correct paths', async ({ page }) => {
    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });
    await page.locator('.sidebar-brand-btn').click();
    await page.waitForSelector('.dashboard', { timeout: 10_000 });

    const games = await page.evaluate(() => {
      return (window as any).__TAURI_INTERNALS__.invoke('get_all_games');
    });

    const hl = games.find((g: any) => g.game_id === 'hogwartslegacy');
    expect(hl).toBeDefined();
    expect(hl.display_name).toBe('Hogwarts Legacy');
    expect(hl.nexus_slug).toBe('hogwartslegacy');
    expect(hl.data_dir).toContain('Phoenix/Content/Paks/~mods');
    expect(hl.exe_path).toContain('Phoenix/Binaries/Win64/HogwartsLegacy.exe');
  });

  test('HL mods have correct metadata', async ({ page }) => {
    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });
    await page.locator('.sidebar-brand-btn').click();
    await page.waitForSelector('.dashboard', { timeout: 10_000 });

    const mods = await page.evaluate(() => {
      return (window as any).__TAURI_INTERNALS__.invoke('get_installed_mods_summary');
    });

    const hlMods = mods.filter((m: any) => m.game_id === 'hogwartslegacy');
    expect(hlMods.length).toBe(5);

    // UE4SS framework
    const ue4ss = hlMods.find((m: any) => m.name === 'RE-UE4SS');
    expect(ue4ss).toBeDefined();
    expect(ue4ss.auto_category).toBe('Framework');
    expect(ue4ss.enabled).toBe(true);
    expect(ue4ss.nexus_mod_id).toBe(942);

    // Collection-sourced mod
    const charEditor = hlMods.find((m: any) => m.name === 'Character Editor');
    expect(charEditor).toBeDefined();
    expect(charEditor.collection_name).toBe('The Goblet');

    // Mod Merger output loads last
    const merger = hlMods.find((m: any) => m.name === 'Hogwarts Mod Merger Output');
    expect(merger).toBeDefined();
    expect(merger.install_priority).toBe(99);
  });

  test('installed HL collection has correct metadata', async ({ page }) => {
    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });
    await page.locator('.sidebar-brand-btn').click();
    await page.waitForSelector('.dashboard', { timeout: 10_000 });

    const collections = await page.evaluate(() => {
      return (window as any).__TAURI_INTERNALS__.invoke('list_installed_collections');
    });

    expect(collections.length).toBe(1);
    expect(collections[0].slug).toBe('uehwil');
    expect(collections[0].name).toBe('The Goblet');
    expect(collections[0].game_domain).toBe('hogwartslegacy');
    expect(collections[0].mod_count).toBe(132);
    expect(collections[0].author).toBe('v2');
  });
});
