// Emscripten JS library of the boundary spike: declares boundary.c's imports.
// The driver (driver.mjs) replaces bnd_run, bnd_translate and green_block in
// the instantiateWasm hook with the translated module's raw `run`, a plain JS
// function and a WebAssembly.Suspending, so these bodies never run.
addToLibrary({
  bnd_run: () => { throw new Error('bnd_run: the driver replaces this import'); },
  bnd_translate: () => { throw new Error('bnd_translate: the driver replaces this import'); },
  green_block: () => { throw new Error('green_block: the driver replaces this import'); },
  // Plain import after C's memory.grow: refresh the glue's views, then the
  // embedder's (Module.onMemoryGrown).
  memory_grown: () => {
    updateMemoryViews();
    Module['onMemoryGrown']?.();
  },
  // Reads a C string through the glue's own views (UTF8ToString).
  bnd_glue_string__deps: ['$UTF8ToString'],
  bnd_glue_string__sig: 'vp',
  bnd_glue_string: (s) => { Module['onGlueString']?.(UTF8ToString(s)); },
});
