import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './',
  use: {
    browserName: 'chromium',
    headless: true,
    args: [
      '--no-sandbox',
      '--disable-setuid-sandbox',
      '--disable-dev-shm-usage',
      '--disable-web-security',
      '--disable-features=VizDisplayCompositor'
    ]
  },
  projects: [
    {
      name: 'chromium',
      use: { 
        browserName: 'chromium',
        args: [
          '--no-sandbox',
          '--disable-setuid-sandbox',
          '--disable-dev-shm-usage',
          '--disable-web-security',
          '--disable-features=VizDisplayCompositor'
        ]
      }
    }
  ]
});
