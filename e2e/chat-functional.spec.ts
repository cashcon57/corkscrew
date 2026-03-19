import { test, expect } from './fixtures/test-fixtures';

test.describe('Chat Panel (functional)', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });
  });

  test('chat shows setup area with backend toggle when no model loaded', async ({ page }) => {
    await page.locator('.chat-toggle-btn').click();
    await expect(page.locator('.chat-container')).toBeVisible({ timeout: 5_000 });

    // Should show the setup area since no model is loaded
    await expect(page.locator('.setup-area')).toBeVisible({ timeout: 5_000 });
    // Backend toggle should be present
    await expect(page.locator('.backend-toggle')).toBeVisible();
  });

  test('backend toggle has MLX and Ollama options', async ({ page }) => {
    await page.locator('.chat-toggle-btn').click();
    await expect(page.locator('.chat-container')).toBeVisible({ timeout: 5_000 });

    // Backend buttons should be visible
    const backendBtns = page.locator('.backend-btn');
    const count = await backendBtns.count();
    expect(count).toBeGreaterThanOrEqual(2); // At least MLX + Ollama
  });

  test('crash badge appears when crashes detected', async ({ page }) => {
    await page.addInitScript(`
      const origInvoke = window.__TAURI_INTERNALS__.invoke;
      window.__TAURI_INTERNALS__.invoke = function(cmd, args) {
        if (cmd === 'chat_check_new_crashes') {
          return Promise.resolve({ count: 2, entries: [
            { filename: "crash-2026-03-18.txt", timestamp: "2026-03-18T14:32:45Z", summary: "ACCESS_VIOLATION", severity: "critical" },
            { filename: "crash-2026-03-18-2.txt", timestamp: "2026-03-18T14:28:12Z", summary: "STACK_OVERFLOW", severity: "critical" },
          ]});
        }
        return origInvoke.call(this, cmd, args);
      };
    `);
    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });

    await expect(page.locator('.chat-crash-badge')).toBeVisible({ timeout: 5_000 });
  });

  test('crash badge hidden when no crashes', async ({ page }) => {
    await expect(page.locator('.chat-crash-badge')).not.toBeVisible();
  });

  test('chat header renders with label', async ({ page }) => {
    await page.locator('.chat-toggle-btn').click();
    await expect(page.locator('.chat-header')).toBeVisible({ timeout: 5_000 });
    await expect(page.locator('.chat-header-label')).toBeVisible();
  });
});
