// The Win32 shim: one module per DLL (docs/design/runtime.md). DLL names
// compare case-insensitively; function names are exact. kernel32 is split by
// area: kernel32.mjs (M2-M4), kernel32-files.mjs and kernel32-system.mjs (M5).
import kernel32 from './kernel32.mjs';
import kernel32Files from './kernel32-files.mjs';
import kernel32System from './kernel32-system.mjs';
import msvcrt from './msvcrt.mjs';
import advapi32 from './advapi32.mjs';
import oleaut32 from './oleaut32.mjs';
import user32 from './user32.mjs';

function merge(...parts) {
  const out = {};
  for (const p of parts) {
    for (const [k, v] of Object.entries(p)) {
      if (Object.hasOwn(out, k)) throw new Error(`shim function ${k} is defined twice`);
      out[k] = v;
    }
  }
  return out;
}

export const SHIM = {
  'kernel32.dll': merge(kernel32, kernel32Files, kernel32System),
  'msvcrt.dll': msvcrt,
  'advapi32.dll': advapi32,
  'oleaut32.dll': oleaut32,
  'user32.dll': user32,
};

/** The shim entry {args, fn} for `dll!name`, or undefined. */
export function findShim(dll, name, shim = SHIM) {
  const mod = shim[dll.toLowerCase()];
  return mod && Object.hasOwn(mod, name) ? mod[name] : undefined;
}
