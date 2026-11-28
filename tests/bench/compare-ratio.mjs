// Compares a bench result's translated/native ratio with the last recorded
// line of the same host (bench.yml, on pull requests; a warning only): prints
// both, and a GitHub warning when the ratio fell more than 10 %. Only a line
// of the same host compares: Phase 1's lines (the shim) have no `host`, the
// Wine host's say "wine".
//   node tests/bench/compare-ratio.mjs RESULT.json HISTORY.jsonl TITLE
import { readFileSync } from 'node:fs';

const [resultPath, historyPath, title] = process.argv.slice(2);
if (!title) {
  console.error('usage: node tests/bench/compare-ratio.mjs RESULT.json HISTORY.jsonl TITLE');
  process.exit(2);
}
const now = JSON.parse(readFileSync(resultPath, 'utf8'));
const history = readFileSync(historyPath, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
const last = history.filter((l) => (l.host ?? null) === (now.host ?? null)).at(-1);
if (!last) {
  console.log(`no line of host ${now.host ?? '(none)'} in ${historyPath} yet; nothing to compare with.`);
} else if (!last.ratio || !now.ratio) {
  console.log('a line has no ratio; skipping');
} else {
  const change = now.ratio / last.ratio - 1;
  console.log(`ratio ${now.ratio} now, ${last.ratio} at ${last.commit} (${(change * 100).toFixed(1)} %)`);
  if (change < -0.10) {
    console.log(`::warning title=${title} ratio::translated/native ratio fell ${(-change * 100).toFixed(1)} % (${last.ratio} at ${last.commit} -> ${now.ratio})`);
  }
}
