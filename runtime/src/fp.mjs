// The floating-point helper module x64wasm_fp.wasm (docs/design/abi.md, "Floating-point
// helper module", v4): instantiated once per process; its exports are the `fp` import
// namespace of every translated module the Linker links.

export const FP_NAMESPACE = 'fp';

/**
 * Instantiates x64wasm_fp.wasm (bytes or a compiled module). It must import nothing.
 * @returns {Promise<WebAssembly.Exports>} the exports, to pass as `fp` to the Linker
 */
export async function loadFp(source) {
  const module = source instanceof WebAssembly.Module ? source : await WebAssembly.compile(source);
  const imports = WebAssembly.Module.imports(module);
  if (imports.length) {
    throw new Error(`floating-point helper module imports ${imports.map((i) => `${i.module}.${i.name}`).join(', ')}; it must import nothing`);
  }
  return (await WebAssembly.instantiate(module, {})).exports;
}

/** The `fp.*` function names a compiled module imports. */
export function fpImports(module) {
  return WebAssembly.Module.imports(module).filter((i) => i.module === FP_NAMESPACE).map((i) => i.name);
}
