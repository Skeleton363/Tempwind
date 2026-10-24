// Hand-built ABI v3 modules for linker and process tests that run before the
// v3 emitter exists; the builder lives in runtime/src/wasm-mini.mjs (the
// browser self-test uses it too).
export { v3Module, blocksPayload } from '../../runtime/src/wasm-mini.mjs';
