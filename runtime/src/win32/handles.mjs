// The process's handle table (runtime.md, "Handles"): files, find handles,
// events, semaphores and file mappings in one table, numbered from 0x100 in
// steps of 4. The standard handles stay fixed (0x3, 0x7, 0xB); the current
// process is -1 and the current thread -2. An unknown handle is an error stop.
import { STDIN_HANDLE, STDOUT_HANDLE, STDERR_HANDLE } from '../abi.mjs';
import { hex } from './util.mjs';

export const FIRST_HANDLE = 0x100n;
export const CURRENT_PROCESS = 0xffff_ffff_ffff_ffffn;
export const CURRENT_THREAD = 0xffff_ffff_ffff_fffen;

const STD = new Map([
  [STDIN_HANDLE, { kind: 'std', stream: 'stdin' }],
  [STDOUT_HANDLE, { kind: 'std', stream: 'stdout' }],
  [STDERR_HANDLE, { kind: 'std', stream: 'stderr' }],
]);

export const handleTable = (ctx) => ctx.dllState('tempwind:handles', () => ({ map: new Map(), next: FIRST_HANDLE }));

/** Adds an object and returns its new handle. */
export function addHandle(ctx, obj) {
  const t = handleTable(ctx);
  const h = t.next;
  t.next += 4n;
  t.map.set(h, obj);
  return h;
}

/**
 * The object behind handle `h`, which must be one of `kinds` ('std', 'file',
 * 'find', 'event', 'semaphore', 'mapping'); any other handle stops.
 */
export function getHandle(ctx, h, api, ...kinds) {
  h = BigInt.asUintN(64, BigInt(h));
  const obj = STD.get(h) ?? handleTable(ctx).map.get(h);
  if (!obj) ctx.fail(`${api}: unknown handle ${hex(h)}`);
  if (kinds.length && !kinds.includes(obj.kind)) ctx.fail(`${api}: handle ${hex(h)} is a ${obj.kind} handle, not ${kinds.join(' or ')}`);
  return obj;
}

/** Removes handle `h` from the table and returns its object (std handles can't be removed). */
export function removeHandle(ctx, h, api) {
  h = BigInt.asUintN(64, BigInt(h));
  const t = handleTable(ctx);
  const obj = t.map.get(h);
  if (!obj) ctx.fail(`${api}: unknown handle ${hex(h)}`);
  t.map.delete(h);
  return obj;
}

export const isStdHandle = (h) => STD.has(BigInt.asUintN(64, BigInt(h)));
