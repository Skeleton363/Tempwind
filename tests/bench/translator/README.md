# Translator measurements (V8)

The V8 half of the M1 translator measurements in `docs/design/translator.md`; the wasmtime half is `crates/testkit/tests/spike_bench.rs`. Results are recorded in `docs/status/phase-1.md`.

```text
cargo run -p testkit --release -- spike --bench --out target/bench/translator
node tests/bench/translator/bench.mjs [--runs N] [--configs default,eager,turbofan,liftoff]
node tests/bench/translator/bench.mjs --browser [PATH]
```

`testkit spike --bench` writes the modules and a manifest in the spike format (`docs/design/abi.md`), plus each module's translation options and, for the timed cases, the native time measured on the same machine (`nativeMs`). It checks every module in wasmtime first.

| Case | What |
|---|---|
| `region_<n>` | Phase 0's generated 201k-instruction region, translated with `max_function_insns = n` (500 … 20000). The run executes only its small function `f`, so "first run" is the time to the first call into a large module. |
| `sum_loop` | `rax = 1 + … + 10^8`, a tight loop |
| `fib_rec` | recursive fib(30), C-compiler style (frame pointer, shadow space) |
| `byte_calls` | a leaf function called once per byte of a 1 MiB buffer (CoreMark's `crcu8` pattern) |
| `deep_probe`, `deep_fat` | recursion translated without the call-depth guard; `deep_fat` runs through a 2000-instruction function at every level |

`bench.mjs` runs everything once per V8 configuration, each in a child process:

- `default`: lazy compilation, Liftoff first, dynamic tier-up to TurboFan (what users get);
- `eager`: `--no-wasm-lazy-compilation`, Liftoff for every function at compile time;
- `turbofan`: `--no-liftoff --no-wasm-lazy-compilation`, TurboFan for every function at compile time;
- `liftoff`: `--liftoff-only`, never tiers up.

It compiles with the synchronous `WebAssembly.Module` constructor: under `--no-wasm-lazy-compilation`, Node doesn't keep its event loop alive for an asynchronous compile, so `await WebAssembly.compile` never settles. A fresh custom section in every copy keeps V8 from reusing a cached module.

`--browser` runs the recursion-depth probes in headless Chromium, in the page and in a module Worker. V8 flags given to Chromium with `--js-flags` did not take effect in Playwright's headless shell, so the browser numbers are for the default configuration only.
