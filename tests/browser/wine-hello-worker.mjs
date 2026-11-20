// Worker for wine-hello.spec: hello.exe on Wine in a module Worker, through the
// browser host (runtime/src/wine/web-files.mjs; wine-unix.md, section 8).
import { loadWebWineInputs } from '/runtime/src/wine/web-files.mjs';
import { createWineProcess } from '/runtime/src/wine/wine-process.mjs';

self.onmessage = async () => {
  try {
    const inputs = await loadWebWineInputs({
      wine: '/target/wine/',
      unix: '/target/wine-unix/',
      rt: '/target/wasm32-unknown-unknown/release/x64wasm_rt.wasm',
      fp: '/target/wasm32-unknown-unknown/release/x64wasm_fp.wasm',
      overlay: '/native/wine/prefix/overlay.reg',
    });
    const res = await fetch('/target/wine/hello.exe');
    if (!res.ok) throw new Error(`hello.exe: HTTP ${res.status} (run tests/wine/prepare.sh)`);
    const bytes = new Uint8Array(await res.arrayBuffer());
    const out = { stdout: '', stderr: '' };
    const o = new TextDecoder(), e = new TextDecoder();
    const proc = await createWineProcess({
      ...inputs,
      program: { name: 'hello.exe', bytes },
      stdout: (b) => { out.stdout += o.decode(b, { stream: true }); },
      stderr: (b) => { out.stderr += e.decode(b, { stream: true }); },
    });
    let status, error;
    try {
      status = await proc.run();
    } catch (err) {
      error = String(err?.stack ?? err);
    }
    const s = proc.stats;
    self.postMessage({
      isolated: self.crossOriginIsolated,
      status, error, stdout: out.stdout, stderr: out.stderr,
      programPath: proc.programPath,
      aot: s.aot.map((a) => a.path),
      imageFallbacks: s.imageFallbacks,
      translations: s.translations.length,
      bootMs: s.bootMs,
      lazy: s.lazy,
    });
  } catch (err) {
    self.postMessage({ error: String(err?.stack ?? err) });
  }
};
