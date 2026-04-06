import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: process.env.CI ? 4 : undefined,
  globalTimeout: process.env.CI ? 30 * 60 * 1000 : undefined,
  timeout: 30_000,
  reporter: [['html', { open: 'never' }]],

  use: {
    baseURL: 'http://localhost:1420',
    screenshot: 'only-on-failure',
    trace: 'on-first-retry',
    actionTimeout: 10_000,
  },

  projects: [
    {
      name: 'webkit',
      use: { ...devices['Desktop Safari'] },
    },
    // Chromium available for local debugging but not run in CI
    // (Corkscrew renders in WebKit via Tauri, not Chrome)
    ...(!process.env.CI ? [{
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    }] : []),
  ],

  webServer: {
    command: 'npm run dev',
    url: 'http://localhost:1420',
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
  },
});
