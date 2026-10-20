// Worker entry: receives {base, opts}, runs the manifest, posts {results}|{error}.
import { runInPage } from './browser-host.mjs';

self.onmessage = async (e) => {
  try {
    const results = await runInPage(e.data.base, e.data.opts);
    self.postMessage({ results });
  } catch (err) {
    self.postMessage({ error: String(err?.stack ?? err) });
  }
};
