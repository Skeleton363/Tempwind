// The Win32 shim: one module per DLL (docs/design/runtime.md). DLL names
// compare case-insensitively; function names are exact.
import kernel32 from './kernel32.mjs';

export const SHIM = {
  'kernel32.dll': kernel32,
};

/** The shim entry {args, fn} for `dll!name`, or undefined. */
export function findShim(dll, name, shim = SHIM) {
  const mod = shim[dll.toLowerCase()];
  return mod && Object.hasOwn(mod, name) ? mod[name] : undefined;
}
