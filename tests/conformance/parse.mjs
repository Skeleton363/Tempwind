// One conformance unit's output and exit code -> the result object of
// docs/design/wine-build.md ("Conformance baselines", results file schema 1).
//
// The summary line of Wine's test framework (include/wine/test.h):
//   <pid>:<unit>:<elapsed> <n> tests executed (<t> marked as todo, <f> as flaky, <e> failure(s)), <s> skipped.
// Every such line is summed, child processes included. <e> counts the todo
// failures too, as the exit code does.

const SUMMARY =
  /^[0-9a-f]{4,}:[^\s:]+:.*?(\d+) tests executed \((\d+) marked as todo, (\d+) as flaky, (\d+) failures?\), (\d+) skipped\.\s*$/;
const UNSUPPORTED = /^tempwind: unsupported:/m;
const EXCEPTION_EXIT = 0xc0000000;

/**
 * @param {{unit: string, output: string, exit: number|null, timedOut?: boolean, seconds?: number}} run
 *   output is stdout and stderr; exit is the process exit code (null: killed by a signal).
 */
export function parseUnit({ unit, output, exit, timedOut = false, seconds = 0 }) {
  let executed = 0, todo = 0, flaky = 0, failures = 0, skipped = 0, lines = 0;
  for (const line of output.split(/\r?\n/)) {
    const m = SUMMARY.exec(line);
    if (!m) continue;
    lines++;
    executed += +m[1]; todo += +m[2]; flaky += +m[3]; failures += +m[4]; skipped += +m[5];
  }
  const code = exit === null || exit === undefined ? -1 : exit >>> 0;
  let status;
  if (timedOut) status = 'timeout';
  else if (UNSUPPORTED.test(output)) status = 'unsupported';
  else if (lines === 0 || exit === null || code >= EXCEPTION_EXIT) status = 'crashed';
  else if (failures > 0) status = 'failed';
  else status = code === 0 ? 'ok' : 'failed';
  return {
    unit, status, executed, todo, flaky, failures, skipped,
    exit: exit === null || exit === undefined ? -1 : code,
    seconds: Math.round(seconds * 1000) / 1000,
  };
}

/** Unit names from `<exe> --list`: the lines after "Valid test names:". */
export function parseList(text) {
  const out = [];
  let seen = false;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!seen) { seen = line.startsWith('Valid test names'); continue; }
    if (line) out.push(line);
  }
  return out;
}
