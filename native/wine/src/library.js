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
  // (state i64) -> i32: a translated module's raw `run` (M3).
  tw_run__sig: 'ij',
  tw_run: (state) => { throw new Error('tw_run: the JS host replaces this import'); },
});
