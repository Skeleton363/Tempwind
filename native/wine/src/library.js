// The imports of Wine's Unix side (docs/design/abi.md v5, "Host entries of
// the Unix side"; docs/design/wine-unix.md, section 1). Emscripten --js-library.
//
// The JS host (runtime/src/wine/unix-host.mjs) replaces green_block (with a
// WebAssembly.Suspending) and tw_run in the instantiateWasm hook, so their
// bodies here never run. green_wake and memory_grown need the glue's scope,
// so they stay library functions and call the host through Module.
//
// Copyright 2026 the Tempwind authors. LGPL-2.1-or-later (native/wine/COPYING.LIB).
addToLibrary({
  // (thread i32) -> i64: the only switch point; resumes with the value JS passes.
  green_block__sig: 'ji',
  green_block: (thread) => { throw new Error('green_block: the JS host replaces this import'); },
  // (thread i32) -> (): C made the thread runnable; JS resumes it after the
  // current call into the module returns.
  green_wake__sig: 'vi',
  green_wake: (thread) => { Module['onGreenWake'](thread); },
  // () -> (): after C's memory.grow, refresh the glue's views, then the host's.
  memory_grown__sig: 'v',
  memory_grown: () => {
    updateMemoryViews();
    Module['onMemoryGrown']?.();
  },
  // M3's imports (abi.md v5, "The bridge" and "Host entries of the Unix
  // side"). The JS host replaces all of them; the bridge's four with the
  // bridge's raw exports, so those calls are wasm to wasm.
  // (state i64) -> i32: the bridge's `run`.
  tw_run__sig: 'ij',
  tw_run: (state) => { throw new Error('tw_run: the JS host replaces this import'); },
  // () -> i32: the bridge's `depth_get`.
  tw_depth_get__sig: 'i',
  tw_depth_get: () => { throw new Error('tw_depth_get: the JS host replaces this import'); },
  // (depth i32) -> (): the bridge's `depth_set`.
  tw_depth_set__sig: 'vi',
  tw_depth_set: (depth) => { throw new Error('tw_depth_set: the JS host replaces this import'); },
  // (state i64) -> (): the bridge's `flags_materialize`.
  tw_flags_materialize__sig: 'vj',
  tw_flags_materialize: (state) => { throw new Error('tw_flags_materialize: the JS host replaces this import'); },
  // (rip i64, base i64, size i64) -> i32: translate from rip inside the
  // executable view and link; 0 when rip then has a block.
  tw_translate__sig: 'ijjj',
  tw_translate: (rip, base, size) => { throw new Error('tw_translate: the JS host replaces this import'); },
  // (fd i32, base i64, size i64) -> (): an image from fd was mapped at base.
  tw_image_mapped__sig: 'vijj',
  tw_image_mapped: (fd, base, size) => { throw new Error('tw_image_mapped: the JS host replaces this import'); },
  // (base i64, size i64) -> (): remove the translations overlapping the range.
  tw_invalidate__sig: 'vjj',
  tw_invalidate: (base, size) => { throw new Error('tw_invalidate: the JS host replaces this import'); },
  // M4's import (abi.md v5, "Green threads"; wine-unix.md v3, section 9).
  // (thread i32, top i64, bottom i64) -> (): C made a green thread whose
  // shadow stack is [bottom, top); JS starts it with tw_thread_main(thread)
  // after the current call into the module returns, in order with the wakes.
  green_spawn__sig: 'vijj',
  green_spawn: (thread, top, bottom) => { Module['onGreenSpawn'](thread, top, bottom); },
});
