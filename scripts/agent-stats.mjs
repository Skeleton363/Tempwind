#!/usr/bin/env node
// Summarizes docs/dev/agent-runs.jsonl (method: docs/dev/agent-runs.md): per agent and effort, the runs,
// median tokens, tokens per tool use, rework and findings; and the main session's usage between snapshots.
//   node scripts/agent-stats.mjs [--milestone M6] [FILE]
import { readFileSync } from 'node:fs';
import { parseArgs } from 'node:util';
import { fileURLToPath } from 'node:url';

const { values, positionals } = parseArgs({ options: { milestone: { type: 'string' } }, allowPositionals: true });
const file = positionals[0] ?? fileURLToPath(new URL('../docs/dev/agent-runs.jsonl', import.meta.url));
const rows = readFileSync(file, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l))
  .filter((r) => !values.milestone || r.milestone === values.milestone);
const median = (xs) => { const s = [...xs].sort((a, b) => a - b); return s.length ? s[Math.floor((s.length - 1) / 2)] : 0; };
const sum = (xs) => xs.reduce((a, b) => a + (b ?? 0), 0);
const k = (n) => `${Math.round(n / 1000)}k`;

const groups = new Map();
for (const r of rows.filter((r) => r.agent !== 'main')) {
  const key = `${r.agent} (${r.model}, ${r.effort})`;
  if (!groups.has(key)) groups.set(key, []);
  groups.get(key).push(r);
}
console.log('agent (model, effort)                  runs  median tok  tok/tool  rework  findings');
for (const [key, rs] of groups) {
  const perTool = sum(rs.map((r) => r.tokens)) / Math.max(1, sum(rs.map((r) => r.tool_uses)));
  const known = (f) => (rs.some((r) => r[f] !== null && r[f] !== undefined) ? String(sum(rs.map((r) => r[f]))) : '-');
  console.log(`${key.padEnd(38)} ${String(rs.length).padStart(4)}  ${k(median(rs.map((r) => r.tokens))).padStart(10)}  ${k(perTool).padStart(8)}  ${known('rework').padStart(6)}  ${known('findings').padStart(8)}`);
}
const main = rows.filter((r) => r.agent === 'main');
for (let i = 1; i < main.length; i++) {
  const a = main[i - 1], b = main[i];
  console.log(`main session ${a.date} -> ${b.date}: $${(b.cost_usd - a.cost_usd).toFixed(2)}, ${k(b.output_tokens - a.output_tokens)} output, ${k(b.cache_read_tokens - a.cache_read_tokens)} cache reads`);
}
