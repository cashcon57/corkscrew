import { test, expect } from '@playwright/test';

test.describe('Chat Panel', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.waitForSelector('.app-shell', { timeout: 15_000 });
  });

  test('chat panel is hidden by default', async ({ page }) => {
    await expect(page.locator('.chat-container')).not.toBeVisible();
  });

  test('clicking chat toggle shows the chat panel', async ({ page }) => {
    await page.locator('.chat-toggle-btn').click();
    await expect(page.locator('.chat-container')).toBeVisible({ timeout: 5_000 });
  });

  test('clicking chat toggle again hides the chat panel', async ({ page }) => {
    await page.locator('.chat-toggle-btn').click();
    await expect(page.locator('.chat-container')).toBeVisible({ timeout: 5_000 });

    await page.locator('.chat-toggle-btn').click();
    await expect(page.locator('.chat-container')).not.toBeVisible();
  });

  test('chat toggle gets chat-active class when panel is open', async ({ page }) => {
    await page.locator('.chat-toggle-btn').click();
    await expect(page.locator('.chat-toggle-btn')).toHaveClass(/chat-active/);
  });

  test('chat toggle loses chat-active class when panel is closed', async ({ page }) => {
    await page.locator('.chat-toggle-btn').click();
    await page.locator('.chat-toggle-btn').click();
    await expect(page.locator('.chat-toggle-btn')).not.toHaveClass(/chat-active/);
  });

  test('chat panel contains header and messages area', async ({ page }) => {
    await page.locator('.chat-toggle-btn').click();
    await expect(page.locator('.chat-container')).toBeVisible({ timeout: 5_000 });

    await expect(page.locator('.chat-header')).toBeVisible();
    await expect(page.locator('.chat-messages')).toBeVisible();
  });

  test('chat crash badge is not visible by default', async ({ page }) => {
    await expect(page.locator('.chat-crash-badge')).not.toBeVisible();
  });

  test('collapsing sidebar hides an open chat panel', async ({ page }) => {
    // Open chat first
    await page.locator('.chat-toggle-btn').click();
    await expect(page.locator('.chat-container')).toBeVisible({ timeout: 5_000 });

    // Collapse sidebar — chat should disappear
    await page.locator('.sidebar-collapse-btn').click();
    await expect(page.locator('nav.sidebar')).toHaveClass(/collapsed/);
    await expect(page.locator('.chat-container')).not.toBeVisible();
  });
});
