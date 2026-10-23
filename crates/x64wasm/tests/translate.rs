//! Translator tests that need no wasm runtime: module shape and validation,
//! code discovery, flag liveness, and robustness on arbitrary bytes.
//! Execution tests live in `crates/testkit/tests/spike*.rs` (x64wasm can't
//! dev-depend on testkit without a cycle).

use wasmparser::{Payload, TypeRef, Validator, WasmFeatures};
use x64wasm::ir::{Op, Term};
use x64wasm::{CodeRegion, Options, abi};

fn hex(s: &str) -> Vec<u8> {
    let d: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..d.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&d[i..i + 2], 16).unwrap())
        .collect()
}

/// sum_loop: mov eax,0; cmp rcx,0; je done; top: add rax,rcx; sub rcx,1; jne top; done: ret
const SUM_LOOP: &str = "b800000000 4883f900 7409 4801c8 4883e901 75f7 c3";
/// push rbx; mov rbx,rcx; call f; pop rbx; ret; f: lea rax,[rbx+rbx*2]; ret
const CALL_LEA: &str = "53 4889cb e802000000 5b c3 488d045b c3";

fn validate(wasm: &[u8]) {
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(wasm)
        .expect("translated module validates");
}

fn translate(bytes: &[u8], base: u64, opts: &Options) -> x64wasm::Translation {
    x64wasm::translate(CodeRegion { base, bytes }, &[base], opts).unwrap()
}

#[test]
fn module_shape_follows_the_abi() {
    for shared in [false, true] {
        let t = translate(
            &hex(SUM_LOOP),
            0x1_4000_1000,
            &Options {
                shared_memory: shared,
                ..Options::default()
            },
        );
        validate(&t.wasm);
        let mut saw_memory = false;
        let mut saw_run = false;
        for payload in wasmparser::Parser::new(0).parse_all(&t.wasm) {
            match payload.unwrap() {
                Payload::ImportSection(imports) => {
                    for import in imports.into_imports() {
                        let import = import.unwrap();
                        assert_eq!(
                            (import.module, import.name),
                            (abi::MEMORY_MODULE, abi::MEMORY_NAME)
                        );
                        let TypeRef::Memory(m) = import.ty else {
                            panic!("only the memory is imported");
                        };
                        assert!(m.memory64);
                        assert_eq!(m.shared, shared);
                        assert_eq!(m.initial, 1);
                        assert_eq!(m.maximum, shared.then_some(abi::MAX_MEMORY_PAGES));
                        saw_memory = true;
                    }
                }
                Payload::ExportSection(exports) => {
                    let names: Vec<_> = exports.into_iter().map(|e| e.unwrap().name).collect();
                    assert_eq!(names, [abi::RUN_EXPORT]);
                    saw_run = true;
                }
                _ => {}
            }
        }
        assert!(saw_memory && saw_run);
    }
}

#[test]
fn discovers_blocks_and_return_sites() {
    let base = 0x1000;
    let t = translate(&hex(CALL_LEA), base, &Options::default());
    validate(&t.wasm);
    // entry, return site after the call, callee
    assert_eq!(t.blocks, [base, base + 9, base + 11]);
    assert_eq!(t.guest_instructions, 7);
}

#[test]
fn functions_chunks_and_names() {
    let base = 0x1000;
    let t = translate(&hex(CALL_LEA), base, &Options::default());
    validate(&t.wasm);
    // The caller (entry block and return site) and the callee.
    assert_eq!(
        t.functions,
        [
            x64wasm::FunctionInfo {
                addr: base,
                chunks: vec![x64wasm::ChunkInfo {
                    wasm_index: 3,
                    blocks: vec![base, base + 9],
                }],
            },
            x64wasm::FunctionInfo {
                addr: base + 11,
                chunks: vec![x64wasm::ChunkInfo {
                    wasm_index: 4,
                    blocks: vec![base + 11],
                }],
            },
        ]
    );
    let wat = wasmprinter::print_bytes(&t.wasm).unwrap();
    assert!(wat.contains("(func $fn_0x1000 (;3;) (type 2)"));
    assert!(wat.contains("(func $fn_0x100b (;4;) (type 2)"));
    assert!(wat.contains("(table $chunks (;0;) 2 2 funcref)"));
    assert!(wat.contains("(global $depth (;0;) (mut i32)"));
    assert!(t.ir.starts_with("function 0x1000: wasm function 3 (fn_0x1000)\n"));

    // Without the depth guard, calls don't test `depth` (they still count it).
    let unguarded = translate(
        &hex(CALL_LEA),
        base,
        &Options {
            call_depth_limit: None,
            ..Options::default()
        },
    );
    validate(&unguarded.wasm);
    assert!(unguarded.wasm.len() < t.wasm.len());
}

#[test]
fn large_functions_are_cut_into_chunks() {
    // 300 x `add rax, rcx`, then ret: one function, chunks of 100.
    let code = "4801c8".repeat(300) + "c3";
    let opts = Options {
        max_function_insns: 100,
        max_chunk_insns: 1000,
        ..Options::default()
    };
    let t = translate(&hex(&code), 0x1000, &opts);
    validate(&t.wasm);
    // A straight line is one block, which is never cut.
    assert_eq!(t.functions.len(), 1);
    assert_eq!(t.functions[0].chunks.len(), 1);
    // Branches make blocks: a jne over each add (75 03) makes 301
    // one-instruction blocks, cut 100 per chunk.
    let code = "7503 4801c8 ".repeat(150) + "c3";
    let t = translate(&hex(&code), 0x1000, &opts);
    validate(&t.wasm);
    let chunks = &t.functions[0].chunks;
    let sizes: Vec<usize> = chunks.iter().map(|c| c.blocks.len()).collect();
    assert_eq!(sizes, [100, 100, 100, 1]);
    assert_eq!(
        chunks.iter().map(|c| c.wasm_index).collect::<Vec<_>>(),
        [3, 4, 5, 6]
    );
    let wat = wasmprinter::print_bytes(&t.wasm).unwrap();
    assert!(wat.contains("$fn_0x1000_c1"));
    assert!(wat.contains("return_call $fn_0x1000_c1"));
}

#[test]
fn dead_flags_are_marked() {
    let prog = x64wasm::lift(
        CodeRegion {
            base: 0x1000,
            bytes: &hex(SUM_LOOP),
        },
        &[0x1000],
    )
    .unwrap();
    let flags: Vec<(u64, bool)> = prog
        .blocks
        .iter()
        .flat_map(|b| b.insns.iter())
        .flat_map(|i| {
            i.ops.iter().filter_map(move |op| match op {
                Op::Flags { live, .. } => Some((i.addr, *live)),
                _ => None,
            })
        })
        .collect();
    // cmp (read by je), add (overwritten by sub), sub (read by jne)
    assert_eq!(flags, [(0x1005, true), (0x100b, false), (0x100e, true)]);
    assert!(matches!(prog.blocks[2].term, Term::Ret(_)));
}

#[test]
fn entry_outside_region_is_an_error() {
    let err = x64wasm::translate(
        CodeRegion {
            base: 0x1000,
            bytes: &[0xc3],
        },
        &[0x1001],
        &Options::default(),
    )
    .unwrap_err();
    assert!(matches!(err, x64wasm::Error::EntryOutsideRegion(0x1001)));
}

/// Arbitrary bytes from several entries must always produce a valid module.
#[test]
fn arbitrary_bytes_translate_to_valid_modules() {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    // Bias towards opcodes we support so that blocks get long.
    const FAVORED: [u8; 24] = [
        0x48, 0x49, 0x4c, 0x01, 0x03, 0x29, 0x2b, 0x39, 0x3b, 0x89, 0x8b, 0x8d, 0x50, 0x58, 0x6a,
        0x68, 0x74, 0x75, 0x7c, 0xe8, 0xeb, 0xc3, 0x83, 0x81,
    ];
    let mut total_insns = 0;
    for round in 0..300 {
        let len = 16 + (next() % 512) as usize;
        let bytes: Vec<u8> = (0..len)
            .map(|_| {
                let r = next();
                if r % 3 == 0 {
                    FAVORED[(r >> 8) as usize % FAVORED.len()]
                } else {
                    (r >> 16) as u8
                }
            })
            .collect();
        let base = if round % 2 == 0 {
            0x1_4000_1000
        } else {
            0x40_0000
        };
        let entries: Vec<u64> = (0..1 + next() % 4)
            .map(|_| base + next() % len as u64)
            .collect();
        for shared in [false, true] {
            let t = x64wasm::translate(
                CodeRegion {
                    base,
                    bytes: &bytes,
                },
                &entries,
                &Options {
                    shared_memory: shared,
                    ..Options::default()
                },
            )
            .unwrap();
            validate(&t.wasm);
            total_insns += t.guest_instructions;
        }
    }
    assert!(
        total_insns > 1000,
        "only {total_insns} instructions translated"
    );
}
