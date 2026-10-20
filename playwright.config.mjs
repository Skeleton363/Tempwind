import { defineConfig } from '@playwright/test';

const PORT = Number(process.env.TEMPWIND_PORT ?? 4173);

// Playwright finds its own Chromium (PLAYWRIGHT_BROWSERS_PATH in cloud
// sessions, `npx playwright install` elsewhere). CHROME_PATH, when set and
// non-empty, points the tests at another Chrome or Chromium binary instead.
const CHROME_PATH = process.env.CHROME_PATH || undefined;

// WebGPU on SwiftShader (software Vulkan). Verified on Chromium 141 in both
// the headless shell (default) and new headless (channel 'chromium'). Without
// these flags requestAdapter() returns null. --enable-features=Vulkan,
// --use-vulkan, --use-angle were not needed.
const WEBGPU_ARGS = [
  '--enable-unsafe-webgpu',
  '--enable-unsafe-swiftshader',
  '--use-webgpu-adapter=swiftshader',
];

export default defineConfig({
  testDir: 'tests/browser',
  testMatch: '*.spec.mjs',
  timeout: 120_000,
  fullyParallel: false,
  workers: 1,
  reporter: [['list']],
  use: { baseURL: `http://127.0.0.1:${PORT}` },
  webServer: {
    command: `node tests/browser/server.mjs ${PORT}`,
    url: `http://127.0.0.1:${PORT}/tests/browser/index.html`,
    reuseExistingServer: !process.env.CI,
  },
  projects: [
    {
      name: 'chromium',
      testIgnore: /webgpu\.spec/,
      use: { browserName: 'chromium', launchOptions: { executablePath: CHROME_PATH } },
    },
    {
      name: 'webgpu',
      testMatch: /webgpu\.spec/,
      use: {
        browserName: 'chromium',
        launchOptions: { executablePath: CHROME_PATH, args: WEBGPU_ARGS },
      },
    },
  ],
});
