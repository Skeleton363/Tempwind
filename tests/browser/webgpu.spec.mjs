import { test, expect } from '@playwright/test';

// Draws a solid triangle with WGSL into a 64x64 rgba8unorm texture cleared to
// another colour, reads it back, and checks pixels. Runs on SwiftShader (see
// the "webgpu" project in playwright.config.mjs for the launch flags).
test('WebGPU triangle renders on SwiftShader and reads back', async ({ page }, testInfo) => {
  await page.goto('/tests/browser/index.html');
  const r = await page.evaluate(async () => {
    if (!navigator.gpu) return { error: 'navigator.gpu is undefined' };
    const adapter = await navigator.gpu.requestAdapter();
    if (!adapter) return { error: 'requestAdapter() returned null' };
    const info = adapter.info ?? (await adapter.requestAdapterInfo?.());
    const infoStr = JSON.stringify({
      vendor: info?.vendor, architecture: info?.architecture, device: info?.device,
      description: info?.description, isFallbackAdapter: adapter.isFallbackAdapter,
    });
    const device = await adapter.requestDevice();
    const W = 64, H = 64;
    const tex = device.createTexture({
      size: [W, H], format: 'rgba8unorm',
      usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC,
    });
    const shader = device.createShaderModule({ code: `
      @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
        var p = array<vec2f, 3>(vec2f(0.0, 0.9), vec2f(-0.9, -0.9), vec2f(0.9, -0.9));
        return vec4f(p[i], 0.0, 1.0);
      }
      @fragment fn fs() -> @location(0) vec4f { return vec4f(1.0, 0.0, 0.0, 1.0); }` });
    const pipeline = device.createRenderPipeline({
      layout: 'auto',
      vertex: { module: shader, entryPoint: 'vs' },
      fragment: { module: shader, entryPoint: 'fs', targets: [{ format: 'rgba8unorm' }] },
      primitive: { topology: 'triangle-list' },
    });
    const bytesPerRow = 256; // 64*4 = 256, already aligned to the 256-byte requirement
    const buf = device.createBuffer({
      size: bytesPerRow * H, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ,
    });
    const enc = device.createCommandEncoder();
    const pass = enc.beginRenderPass({ colorAttachments: [{
      view: tex.createView(), loadOp: 'clear', storeOp: 'store', clearValue: { r: 0, g: 0, b: 1, a: 1 },
    }] });
    pass.setPipeline(pipeline);
    pass.draw(3);
    pass.end();
    enc.copyTextureToBuffer({ texture: tex }, { buffer: buf, bytesPerRow }, [W, H]);
    device.queue.submit([enc.finish()]);
    await buf.mapAsync(GPUMapMode.READ);
    const data = new Uint8Array(buf.getMappedRange().slice(0));
    buf.unmap();
    const px = (x, y) => Array.from(data.subarray(y * bytesPerRow + x * 4, y * bytesPerRow + x * 4 + 4));
    const reds = (() => { let n = 0; for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) if (px(x, y)[0] > 200) n++; return n; })();
    return {
      infoStr, features: [...adapter.features].length,
      centre: px(32, 40), corners: [px(0, 0), px(63, 0), px(0, 63), px(63, 63)].map((p, i) => ({ i, p })),
      redPixels: reds,
    };
  });
  expect(r.error, r.error).toBeUndefined();
  console.log('adapter info:', r.infoStr, 'red pixels:', r.redPixels);
  testInfo.annotations.push({ type: 'measurement', description: `adapter ${r.infoStr}` });

  const near = (got, want) => got.every((v, i) => Math.abs(v - want[i]) <= 1);
  expect(near(r.centre, [255, 0, 0, 255]), `centre ${r.centre}`).toBe(true);
  // Top corners are outside the triangle (apex is at the top centre); bottom
  // corners are outside too because the triangle's base spans +-0.9, not +-1.
  for (const { i, p } of r.corners) expect(near(p, [0, 0, 255, 255]), `corner ${i} ${p}`).toBe(true);
  expect(r.redPixels).toBeGreaterThan(500); // triangle area is ~ 0.5*58*58 ~ 1700 px
  expect(r.infoStr.toLowerCase()).toContain('swiftshader');
});
