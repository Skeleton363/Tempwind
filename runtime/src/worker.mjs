// Worker entry. Messages:
//   {base, opts}            run the manifest at base, post {results} | {error}
//   {spec, wasm, opts}      run one case from bytes, post {result} | {error}
import { runInPage, withFp } from './browser-host.mjs';
import { runCase } from './run.mjs';

self.onmessage = async (e) => {
  try {
    if (e.data.spec) {
      const { ok, diffs, exit, state, memoryForm } = await runCase(e.data.wasm, e.data.spec, await withFp(e.data.opts));
      self.postMessage({ result: { ok, diffs, exit, memoryForm, rax: state.regs.rax } });
    } else {
      self.postMessage({ results: await runInPage(e.data.base, e.data.opts) });
    }
  } catch (err) {
    self.postMessage({ error: String(err?.stack ?? err) });
  }
};
