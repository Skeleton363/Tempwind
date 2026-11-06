#!/usr/bin/env node
// Real token usage of subagent runs, summed from their transcripts
// (docs/dev/agent-runs.md). A completion notice's token count is only the
// context size of the run's last request; the transcript has every request.
//
//   node scripts/agent-usage.mjs [--agent ID] [--since ISO] [--json] [DIR]
//
// DIR defaults to every session's subagents directory of this project under
// ~/.claude/projects. --since counts only requests from that time on (the
// increment of a resumed run). --json prints one object per run, the fields
// the ledger takes.
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { homedir } from 'node:os';
import { parseArgs } from 'node:util';

const { values, positionals } = parseArgs({
  options: { agent: { type: 'string' }, since: { type: 'string' }, json: { type: 'boolean' } },
  allowPositionals: true,
});

// Price weights relative to an uncached input token, the same for every
// current model: cache writes 1.25, cache reads 0.1, output 5.
const index = (u) => Math.round(u.input + 1.25 * u.cache_write + 0.1 * u.cache_read + 5 * u.output);

function dirs() {
  if (positionals[0]) return [positionals[0]];
  const root = join(homedir(), '.claude/projects', process.cwd().replace(/[/.]/g, '-'));
  if (!existsSync(root)) return [];
  return readdirSync(root, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => join(root, d.name, 'subagents'))
    .filter(existsSync);
}

const runs = [];
for (const dir of dirs()) {
  for (const f of readdirSync(dir).filter((n) => n.endsWith('.jsonl'))) {
    const id = f.slice('agent-'.length, -'.jsonl'.length);
    if (values.agent && id !== values.agent) continue;
    const metaPath = join(dir, `agent-${id}.meta.json`);
    const meta = existsSync(metaPath) ? JSON.parse(readFileSync(metaPath, 'utf8')) : {};
    const requests = new Map();
    const models = new Map(); // the model that served each request (meta's pickedEffortModel is the parent's)
    for (const line of readFileSync(join(dir, f), 'utf8').split('\n')) {
      if (!line) continue;
      let e;
      try { e = JSON.parse(line); } catch { continue; }
      const m = e.message;
      if (m?.role !== 'assistant' || !m.usage) continue;
      if (values.since && e.timestamp < values.since) continue;
      if (m.model) models.set(m.model, (models.get(m.model) ?? 0) + 1);
      requests.set(m.id ?? e.uuid, { u: m.usage, t: e.timestamp }); // a request logs once per content block
    }
    if (!requests.size) continue;
    const u = { input: 0, cache_write: 0, cache_read: 0, output: 0 };
    let first = null, last = null;
    for (const { u: r, t } of requests.values()) {
      u.input += r.input_tokens ?? 0;
      u.cache_write += r.cache_creation_input_tokens ?? 0;
      u.cache_read += r.cache_read_input_tokens ?? 0;
      u.output += r.output_tokens ?? 0;
      if (!first || t < first) first = t;
      if (!last || t > last) last = t;
    }
    const model = [...models].sort((a, b) => b[1] - a[1])[0]?.[0];
    runs.push({
      id, task: meta.description ?? '', agent: meta.agentType ?? '', model: model ?? '', effort: meta.effort ?? 'default',
      requests: requests.size, ...u, index: index(u), first, last,
    });
  }
}
runs.sort((a, b) => (a.first < b.first ? -1 : 1));
if (values.json) {
  for (const r of runs) console.log(JSON.stringify(r));
} else {
  const M = (n) => (n / 1e6).toFixed(2) + 'M';
  console.log('first (UTC)       agent        effort  req  cache_read  cache_write  output   index  task');
  for (const r of runs) {
    console.log(
      `${r.first.slice(0, 16)}  ${r.agent.padEnd(12).slice(0, 12)} ${r.effort.padEnd(7)} ${String(r.requests).padStart(4)}  ${M(r.cache_read).padStart(10)}  ${M(r.cache_write).padStart(11)}  ${M(r.output).padStart(6)}  ${M(r.index).padStart(6)}  ${r.task.slice(0, 50)}`,
    );
  }
}
