#!/usr/bin/env node
// The coordinator's context, from the session's transcript (trial 6,
// docs/dev/agent-runs.md). Per compaction segment: its requests, their mean
// and largest context (input + cache writes + cache reads), and their cache
// reads. Then the last request's context, which the next request re-reads.
// Exact and without lag, unlike get_session's context_usage, which leaves out
// about 75k tokens and updates only at a turn's start. Subagents' requests are
// left out (isSidechain; theirs are in their own transcripts).
//
//   node scripts/context-usage.mjs [--json] [TRANSCRIPT.jsonl]
//
// TRANSCRIPT defaults to this project's newest session transcript under
// ~/.claude/projects.
import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { homedir } from 'node:os';
import { parseArgs } from 'node:util';

const { values, positionals } = parseArgs({ options: { json: { type: 'boolean' } }, allowPositionals: true });

function transcript() {
  if (positionals[0]) return positionals[0];
  const dir = join(homedir(), '.claude/projects', process.cwd().replace(/[/.]/g, '-'));
  const files = existsSync(dir) ? readdirSync(dir).filter((n) => n.endsWith('.jsonl')).map((n) => join(dir, n)) : [];
  if (!files.length) {
    console.error(`context-usage: no session transcript in ${dir}; pass one`);
    process.exit(1);
  }
  return files.sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0];
}

const file = transcript();
const segments = [];
let cur = null;
const open = (at, why) => { cur = { why, first: at, last: at, requests: new Map() }; segments.push(cur); };
for (const line of readFileSync(file, 'utf8').split('\n')) {
  if (!line) continue;
  let e;
  try { e = JSON.parse(line); } catch { continue; }
  if (e.isSidechain) continue;
  if (e.type === 'system' && e.subtype === 'compact_boundary') { open(e.timestamp, 'compaction'); continue; }
  const m = e.message;
  if (e.type !== 'assistant' || !m?.usage || String(m.model).includes('synthetic')) continue;
  if (!cur) open(e.timestamp, 'start');
  const u = m.usage;
  const context = (u.input_tokens ?? 0) + (u.cache_creation_input_tokens ?? 0) + (u.cache_read_input_tokens ?? 0);
  // A request logs once per content block, with the same id: keep the last.
  cur.requests.set(m.id ?? e.uuid, { context, read: u.cache_read_input_tokens ?? 0, at: e.timestamp });
  cur.last = e.timestamp;
}

const out = segments.map((s, i) => {
  const r = [...s.requests.values()];
  const sum = r.reduce((a, x) => a + x.context, 0);
  return {
    segment: i, after: s.why, first: s.first, last: s.last, requests: r.length,
    mean: r.length ? Math.round(sum / r.length) : 0,
    max: r.reduce((a, x) => Math.max(a, x.context), 0),
    cacheReads: r.reduce((a, x) => a + x.read, 0),
  };
});
const lastSeg = [...segments].reverse().find((s) => s.requests.size);
const lastReq = lastSeg ? [...lastSeg.requests.values()].at(-1) : null;
const now = lastReq ? { tokens: lastReq.context, at: lastReq.at, segment: segments.indexOf(lastSeg) } : null;

if (values.json) {
  console.log(JSON.stringify({ transcript: file, segments: out, now }, null, 2));
} else {
  const k = (n) => `${Math.round(n / 1000)}k`;
  console.log(`transcript: ${file}`);
  for (const s of out) {
    if (!s.requests) continue;
    console.log(`#${s.segment} after ${s.after} ${s.first} .. ${s.last}: ${s.requests} requests, mean ${k(s.mean)}, max ${k(s.max)}, cache reads ${(s.cacheReads / 1e6).toFixed(1)}M`);
  }
  if (now) console.log(`now: ${k(now.tokens)} tokens of context (the last request, ${now.at}, segment #${now.segment})`);
}
