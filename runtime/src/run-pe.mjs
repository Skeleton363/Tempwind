#!/usr/bin/env node
// run-pe: runs a translated 64-bit PE program in Node (docs/design/runtime.md, "CLI").
//
//   node runtime/src/run-pe.mjs --exe FILE.exe --aot FILE.wasm --plan FILE.plan.json [--rt x64wasm_rt.wasm]
//
// stdout and stderr are the guest's, byte for byte, and the exit code is the
// guest's (POSIX keeps its low 8 bits). When the runtime itself stops with an
// error, it prints "tempwind: <error>" to stderr and exits with 125.
// TEMPWIND_TRACE=api,translate,exits traces to stderr.
import { readFile } from 'node:fs/promises';
import { parseArgs } from 'node:util';
import { Process, ProcessError } from './process.mjs';
import { PlanError } from './plan.mjs';
import { RuntimeTranslator } from './rt.mjs';
import { parseTrace } from './trace.mjs';

export const RUNTIME_ERROR_EXIT = 125;
const USAGE = 'usage: run-pe --exe FILE.exe --aot FILE.wasm --plan FILE.plan.json [--rt x64wasm_rt.wasm]';

class UsageError extends Error {}

async function main(argv) {
  let values;
  try {
    ({ values } = parseArgs({
      args: argv,
      options: { exe: { type: 'string' }, aot: { type: 'string' }, plan: { type: 'string' }, rt: { type: 'string' } },
      strict: true, allowPositionals: false,
    }));
  } catch (e) {
    throw new UsageError(`${e.message}\n${USAGE}`);
  }
  for (const k of ['exe', 'aot', 'plan']) if (!values[k]) throw new UsageError(`missing --${k}\n${USAGE}`);
  let trace;
  try { trace = parseTrace(process.env.TEMPWIND_TRACE ?? ''); } catch (e) { throw new UsageError(e.message); }
  const read = async (path, what) => {
    try { return new Uint8Array(await readFile(path)); } catch (e) { throw new UsageError(`cannot read ${what} ${path}: ${e.message}`); }
  };
  const exe = await read(values.exe, 'the program');
  const aot = await read(values.aot, 'the AOT module');
  let plan;
  try { plan = JSON.parse(new TextDecoder().decode(await read(values.plan, 'the load plan'))); } catch (e) {
    throw e instanceof UsageError ? e : new UsageError(`the load plan ${values.plan} is not JSON: ${e.message}`);
  }
  let rt = null;
  if (values.rt) {
    try { rt = await RuntimeTranslator.load(await read(values.rt, 'the run-time translator')); } catch (e) {
      throw e instanceof UsageError ? e : new UsageError(`cannot load the run-time translator ${values.rt}: ${e.message}`);
    }
  }
  const p = await Process.load({
    exe, plan, aot, rt, trace,
    log: (line) => process.stderr.write(`[tempwind] ${line}\n`),
    onOutput: (stream, bytes) => (stream === 'stdout' ? process.stdout : process.stderr).write(bytes),
  });
  return p.start();
}

const known = (e) => e instanceof ProcessError || e instanceof PlanError || e instanceof UsageError;

try {
  process.exitCode = await main(process.argv.slice(2));
} catch (e) {
  process.stderr.write(`tempwind: ${known(e) ? e.message : e?.stack ?? e}\n`);
  process.exitCode = RUNTIME_ERROR_EXIT;
}
