// TEMPWIND_TRACE (docs/design/runtime.md, "Tracing"): a comma-separated list.
export const TRACE_KINDS = ['api', 'translate', 'exits'];

/** @returns {{api: boolean, translate: boolean, exits: boolean}} */
export function parseTrace(value = '') {
  const out = Object.fromEntries(TRACE_KINDS.map((k) => [k, false]));
  for (const raw of value.split(',')) {
    const k = raw.trim();
    if (!k) continue;
    if (!TRACE_KINDS.includes(k)) throw new Error(`TEMPWIND_TRACE: unknown value "${k}" (known: ${TRACE_KINDS.join(', ')})`);
    out[k] = true;
  }
  return out;
}
