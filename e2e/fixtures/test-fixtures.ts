import { test as base, expect } from '@playwright/test';
import { injectTauriMock } from './tauri-mock';

/**
 * Extended test fixture that injects the Tauri mock bridge before each test.
 * Import `{ test, expect }` from this file for functional tests that need
 * realistic mock data from the "backend".
 */
export const test = base.extend<{ mockOverrides: Record<string, unknown> }>({
  mockOverrides: [{}, { option: true }],
  page: async ({ page, mockOverrides }, use) => {
    await injectTauriMock(page, mockOverrides);
    await use(page);
  },
});

export { expect };
