import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  fullyParallel: false, // Run tests sequentially (shared state from beforeAll)
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  reporter: [
    ['html', { open: 'never' }],
    ['list'],
  ],
  timeout: 120000, // 2 minutes for compilation + execution
  expect: {
    timeout: 5000,
  },
  use: {
    trace: 'on-first-retry',
  },
});
