import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { runSpikeDir, formatResults } from '../../runtime/src/node-host.mjs';

const dir = new URL('../../target/spike/', import.meta.url).pathname;

test('translator spike cases pass in Node', async () => {
  assert.ok(
    existsSync(dir + 'manifest.json'),
    'target/spike/manifest.json is missing. Create it with: cargo run -p testkit --release -- spike --out target/spike',
  );
  const results = await runSpikeDir(dir);
  console.log(formatResults(results));
  assert.ok(results.length > 0, 'manifest has no cases');
  assert.deepEqual(results.filter((r) => !r.ok).map((r) => r.name), []);
});
