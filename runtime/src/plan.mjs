// The load plan that `translate-pe` writes (docs/design/runtime.md, "Load
// plan"), its checks against the PE file, and the Phase 1 address map.

import {
  PAGE_SIZE, GUEST_HEAP_BASE, STACK_TOP, PEB_ADDR, PEB_SIZE, TEB_ADDR, TEB_SIZE,
  KUSER_SHARED_DATA, KUSER_SIZE, HOST_BASE, HOST_END, RUNTIME_HEAP_BASE, NULL_REGION_SIZE,
  ADDRESS_LIMIT, STACK_ALIGN, MAX_THUNKS, IMAGE_FILE_MACHINE_AMD64, IMAGE_SCN_MEM_EXECUTE,
} from './abi.mjs';

export const PLAN_VERSION = 1;
const PAGE = BigInt(PAGE_SIZE);
export const hex = (v) => '0x' + BigInt(v).toString(16);
const roundUp = (v, a) => (v + a - 1n) / a * a;

export class PlanError extends Error {}

function u64(obj, key, where) {
  const v = obj?.[key];
  if (typeof v !== 'string' || !/^0x[0-9a-fA-F]{1,16}$/.test(v)) {
    throw new PlanError(`plan: ${where}.${key} must be a "0x…" u64 string, got ${JSON.stringify(v)}`);
  }
  return BigInt(v);
}

/**
 * Validates the plan's shape and converts its u64 strings to BigInt.
 * @param {object} json the parsed `<stem>.plan.json`
 */
export function parsePlan(json) {
  if (json?.version !== PLAN_VERSION) throw new PlanError(`plan: version ${JSON.stringify(json?.version)}, expected ${PLAN_VERSION}`);
  if (typeof json.sha256 !== 'string' || !/^[0-9a-fA-F]{64}$/.test(json.sha256)) throw new PlanError('plan: sha256 must be 64 hex digits');
  if (!Array.isArray(json.sections)) throw new PlanError('plan: sections must be an array');
  if (!Array.isArray(json.imports)) throw new PlanError('plan: imports must be an array');
  const plan = {
    file: String(json.file ?? ''),
    sha256: json.sha256.toLowerCase(),
    machine: Number(u64(json, 'machine', 'plan')),
    imageBase: u64(json, 'imageBase', 'plan'),
    sizeOfImage: u64(json, 'sizeOfImage', 'plan'),
    sizeOfHeaders: u64(json, 'sizeOfHeaders', 'plan'),
    entryPoint: u64(json, 'entryPoint', 'plan'),
    subsystem: json.subsystem,
    stackReserve: u64(json, 'stackReserve', 'plan'),
    stackCommit: u64(json, 'stackCommit', 'plan'),
    hasRelocations: !!json.hasRelocations,
    sections: json.sections.map((s, i) => {
      const w = `sections[${i}]`;
      const sec = {
        name: String(s.name ?? ''),
        va: u64(s, 'va', w),
        virtualSize: u64(s, 'virtualSize', w),
        fileOffset: u64(s, 'fileOffset', w),
        fileSize: u64(s, 'fileSize', w),
        characteristics: Number(u64(s, 'characteristics', w)),
      };
      // As in x64wasm::pe: a zero VirtualSize means the raw size.
      sec.mappedSize = sec.virtualSize === 0n ? sec.fileSize : sec.virtualSize;
      sec.copySize = sec.fileSize < sec.mappedSize ? sec.fileSize : sec.mappedSize;
      sec.executable = (sec.characteristics & IMAGE_SCN_MEM_EXECUTE) !== 0;
      return sec;
    }),
    imports: json.imports.map((d, i) => {
      if (typeof d.dll !== 'string' || !d.dll) throw new PlanError(`plan: imports[${i}].dll must be a name`);
      if (!Array.isArray(d.functions)) throw new PlanError(`plan: imports[${i}].functions must be an array`);
      return {
        dll: d.dll,
        functions: d.functions.map((f, j) => {
          const w = `imports[${i}].functions[${j}]`;
          const byName = typeof f.name === 'string' && f.name.length > 0;
          const byOrdinal = Number.isInteger(f.ordinal) && f.ordinal >= 0 && f.ordinal <= 0xffff;
          if (byName === byOrdinal || (byName && f.ordinal != null)) {
            throw new PlanError(`plan: ${w} needs exactly one of a name and an ordinal`);
          }
          return { name: byName ? f.name : `#${f.ordinal}`, ordinal: byName ? null : f.ordinal, iat: u64(f, 'iat', w) };
        }),
      };
    }),
    tls: null,
  };
  if (json.tls != null) {
    const t = json.tls;
    if (!Array.isArray(t.callbacks)) throw new PlanError('plan: tls.callbacks must be an array');
    plan.tls = {
      startOfRawData: u64(t, 'startOfRawData', 'tls'),
      endOfRawData: u64(t, 'endOfRawData', 'tls'),
      addressOfIndex: u64(t, 'addressOfIndex', 'tls'),
      sizeOfZeroFill: u64(t, 'sizeOfZeroFill', 'tls'),
      callbacks: t.callbacks.map((c, i) => {
        if (typeof c !== 'string' || !/^0x[0-9a-fA-F]{1,16}$/.test(c)) throw new PlanError(`plan: tls.callbacks[${i}] must be a "0x…" u64 string`);
        return BigInt(c);
      }),
    };
  }
  return plan;
}

const overlaps = (a0, a1, b0, b1) => a0 < b1 && b0 < a1;

/**
 * The Phase 1 address map of a plan, or a PlanError for a bad image base.
 * @returns {{imageBase: bigint, imageEnd: bigint, stackBottom: bigint, stackTop: bigint,
 *   guestHeap: {base: bigint, limit: bigint}, runtimeHeap: {base: bigint, limit: bigint},
 *   memoryBytes: bigint}}
 */
export function addressMap(plan) {
  const { imageBase, sizeOfImage } = plan;
  if (sizeOfImage === 0n) throw new PlanError('plan: sizeOfImage is 0');
  if (imageBase % PAGE) throw new PlanError(`image base ${hex(imageBase)} is not 64 KiB aligned`);
  if (plan.stackReserve === 0n) throw new PlanError('plan: stackReserve is 0');
  const imageEnd = imageBase + sizeOfImage;
  const stackSize = roundUp(plan.stackReserve, STACK_ALIGN);
  const stackBottom = STACK_TOP - stackSize;
  if (stackBottom <= GUEST_HEAP_BASE) throw new PlanError(`stack reserve ${hex(plan.stackReserve)} does not fit below ${hex(STACK_TOP)}`);
  if (imageEnd > ADDRESS_LIMIT) throw new PlanError(`image [${hex(imageBase)}, ${hex(imageEnd)}) ends above 16 GiB`);
  const reserved = [
    ['the NULL region', 0n, NULL_REGION_SIZE],
    ["the guest heap's start", GUEST_HEAP_BASE, GUEST_HEAP_BASE + 1n],
    ['the stack', stackBottom, STACK_TOP],
    ['the PEB', PEB_ADDR, PEB_ADDR + PEB_SIZE],
    ['the TEB', TEB_ADDR, TEB_ADDR + TEB_SIZE],
    ['KUSER_SHARED_DATA', KUSER_SHARED_DATA, KUSER_SHARED_DATA + KUSER_SIZE],
    ['the host region', HOST_BASE, HOST_END],
  ];
  for (const [what, lo, hi] of reserved) {
    if (overlaps(imageBase, imageEnd, lo, hi)) {
      throw new PlanError(`image [${hex(imageBase)}, ${hex(imageEnd)}) overlaps ${what} [${hex(lo)}, ${hex(hi)})`);
    }
  }
  const heapLimit = imageBase > GUEST_HEAP_BASE && imageBase < stackBottom ? imageBase : stackBottom;
  const top = imageEnd > HOST_END ? imageEnd : HOST_END;
  return {
    imageBase, imageEnd, stackBottom, stackTop: STACK_TOP,
    guestHeap: { base: GUEST_HEAP_BASE, limit: heapLimit },
    runtimeHeap: { base: RUNTIME_HEAP_BASE, limit: HOST_END },
    memoryBytes: roundUp(top, PAGE),
  };
}

/** Lowercase hex SHA-256 of `bytes` (WebCrypto: Node and browsers). */
export async function sha256(bytes) {
  const d = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
  return Array.from(d, (b) => b.toString(16).padStart(2, '0')).join('');
}

/**
 * Checks a parsed plan against the PE file it claims to describe, and returns
 * its address map. Throws PlanError with the first problem.
 * @param {ReturnType<typeof parsePlan>} plan
 * @param {Uint8Array} exe
 */
export async function checkPlan(plan, exe) {
  const digest = await sha256(exe);
  if (digest !== plan.sha256) throw new PlanError(`plan sha256 ${plan.sha256} does not match the file (${digest})`);
  if (plan.machine !== IMAGE_FILE_MACHINE_AMD64) throw new PlanError(`machine ${hex(plan.machine)} is not x86-64 (0x8664)`);
  const map = addressMap(plan);
  const fileLen = BigInt(exe.length);
  const inImage = (a, len = 1n) => a >= map.imageBase && a + len <= map.imageEnd;
  if (plan.sizeOfHeaders > plan.sizeOfImage || plan.sizeOfHeaders > fileLen) {
    throw new PlanError(`sizeOfHeaders ${hex(plan.sizeOfHeaders)} exceeds the image or the file`);
  }
  for (const s of plan.sections) {
    if (!inImage(s.va, s.mappedSize)) throw new PlanError(`section ${s.name} [${hex(s.va)}, +${hex(s.mappedSize)}) is outside the image`);
    if (s.fileOffset + s.copySize > fileLen) throw new PlanError(`section ${s.name} file range [${hex(s.fileOffset)}, +${hex(s.copySize)}) is outside the file`);
  }
  if (!executableSection(plan, plan.entryPoint)) throw new PlanError(`entry point ${hex(plan.entryPoint)} is not in an executable section`);
  let count = 0;
  for (const d of plan.imports) {
    for (const f of d.functions) {
      count++;
      if (!inImage(f.iat, 8n) || f.iat % 8n) throw new PlanError(`IAT slot ${hex(f.iat)} of ${d.dll}!${f.name} is not an aligned u64 in the image`);
    }
  }
  if (count > MAX_THUNKS) throw new PlanError(`${count} imported functions, more than the ${MAX_THUNKS} thunks`);
  if (plan.tls) {
    const t = plan.tls;
    if (t.endOfRawData < t.startOfRawData || (t.endOfRawData > t.startOfRawData && !inImage(t.startOfRawData, t.endOfRawData - t.startOfRawData))) {
      throw new PlanError(`TLS template [${hex(t.startOfRawData)}, ${hex(t.endOfRawData)}) is not in the image`);
    }
    if (!inImage(t.addressOfIndex, 4n)) throw new PlanError(`TLS index ${hex(t.addressOfIndex)} is not in the image`);
    for (const c of t.callbacks) {
      if (!executableSection(plan, c)) throw new PlanError(`TLS callback ${hex(c)} is not in an executable section`);
    }
  }
  return map;
}

/** The executable section holding `va`, or undefined. */
export function executableSection(plan, va) {
  return plan.sections.find((s) => s.executable && va >= s.va && va < s.va + s.mappedSize);
}
