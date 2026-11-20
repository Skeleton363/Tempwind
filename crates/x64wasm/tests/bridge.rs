//! The bridge module (`abi.md` v5, "Wine boundary", "The bridge") in
//! wasmtime, with translated modules built for a Wine process: `run`
//! through `env.table[RUN_INDEX]`, the placeholder, `depth_get` and
//! `depth_set`, `flags_materialize` against `run`'s own materialization,
//! the tombstone function, and lookups past tombstones ("Lookup table").

mod common;

use common::{MASK, Process, STATE, hex, translate};
use wasmtime::{Func, Instance, Ref, Val};
use x64wasm::Options;
use x64wasm::abi::{self, flags::STATUS};
use x64wasm::ir::{CC_EAGER, FlagOp, Width};

const BASE: u64 = 0x1_0000;
const RAX: usize = 0;
const RBX: usize = 3;

/// A process with the bridge instantiated (it fills the reserved slots).
fn with_bridge() -> (Process, Instance) {
    let mut p = Process::new();
    let bridge = p.instantiate(&x64wasm::bridge::module(), None);
    (p, bridge)
}

/// Links `code` at `base` and puts the module's `run` at `RUN_INDEX`.
fn link_run(p: &mut Process, code: &str, base: u64) -> Instance {
    let code = hex(code);
    p.write(base, &code);
    let module = p.link(&translate(&code, base, &Options::default()));
    let run = module.get_func(&mut p.store, abi::RUN_EXPORT).unwrap();
    p.table
        .set(&mut p.store, abi::RUN_INDEX.into(), Ref::Func(Some(run)))
        .unwrap();
    module
}

fn table_func(p: &mut Process, index: u32) -> Func {
    match p.table.get(&mut p.store, index.into()).unwrap() {
        Ref::Func(Some(f)) => f,
        other => panic!("table slot {index}: {other:?}"),
    }
}

/// The bridge validates, and its imports and exports are the contract's.
#[test]
fn bridge_shape() {
    let wasm = x64wasm::bridge::module();
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&wasm)
        .unwrap();
    let (mut imports, mut exports) = (Vec::new(), Vec::new());
    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        match payload.unwrap() {
            wasmparser::Payload::ImportSection(r) => {
                for i in r.into_imports() {
                    let i = i.unwrap();
                    imports.push(format!("{}.{} {:?}", i.module, i.name, i.ty));
                }
            }
            wasmparser::Payload::ExportSection(r) => {
                for e in r {
                    exports.push(e.unwrap().name.to_string());
                }
            }
            wasmparser::Payload::CustomSection(c) => {
                assert_ne!(
                    c.name(),
                    "x64wasm.blocks",
                    "nothing in the bridge is linked"
                );
            }
            _ => {}
        }
    }
    assert_eq!(imports.len(), 3, "{imports:?}");
    assert!(imports[0].starts_with("env.memory") && imports[0].contains("shared: true"));
    assert!(imports[0].contains(&format!("maximum: Some({})", abi::MAX_MEMORY_PAGES)));
    assert!(imports[0].contains("memory64: true"));
    assert!(imports[1].starts_with("env.table"));
    assert!(imports[1].contains(&format!("initial: {}", abi::TABLE_RESERVED)));
    assert!(imports[2].starts_with("env.depth") && imports[2].contains("mutable: true"));
    assert_eq!(
        exports,
        [
            abi::RUN_EXPORT,
            abi::BRIDGE_DEPTH_GET_EXPORT,
            abi::BRIDGE_DEPTH_SET_EXPORT,
            abi::BRIDGE_FLAGS_MATERIALIZE_EXPORT,
        ]
    );
}

/// Before a module is linked, `run` reaches the placeholder: it returns
/// EXIT_LEFT_CODE and writes `state.exit`, and nothing else.
#[test]
fn placeholder_leaves_code() {
    let (mut p, bridge) = with_bridge();
    let pattern: Vec<u8> = (0..abi::STATE_SIZE).map(|i| (i * 7 + 3) as u8).collect();
    p.write(STATE, &pattern);
    let depth_before = p.depth.get(&mut p.store).unwrap_i32();
    assert_eq!(p.call_run(&bridge, abi::RUN_EXPORT), abi::EXIT_LEFT_CODE);
    let mut want = pattern;
    want[abi::STATE_EXIT as usize..][..4].copy_from_slice(&abi::EXIT_LEFT_CODE.to_le_bytes());
    assert_eq!(p.read(STATE, abi::STATE_SIZE as usize), want);
    assert_eq!(p.depth.get(&mut p.store).unwrap_i32(), depth_before);
}

/// With a translated module's `run` at `RUN_INDEX`, the bridge's `run` is
/// that trampoline, and keeps the caller's `depth` (v5).
#[test]
fn run_through_the_table_slot() {
    let (mut p, bridge) = with_bridge();
    // mov eax, 42; ret
    link_run(&mut p, "b82a000000 c3", BASE);
    p.depth.set(&mut p.store, Val::I32(5)).unwrap();
    p.enter_state(BASE, &[]);
    assert_eq!(p.call_run(&bridge, abi::RUN_EXPORT), abi::EXIT_LEFT_CODE);
    let s = p.state();
    assert_eq!(
        (s.rip, s.gpr[RAX], s.exit),
        (abi::SENTINEL_RET, 42, abi::EXIT_LEFT_CODE)
    );
    assert_eq!(p.depth.get(&mut p.store).unwrap_i32(), 5);
}

#[test]
fn depth_get_and_set() {
    let (mut p, bridge) = with_bridge();
    let get = bridge
        .get_typed_func::<(), i32>(&mut p.store, abi::BRIDGE_DEPTH_GET_EXPORT)
        .unwrap();
    let set = bridge
        .get_typed_func::<i32, ()>(&mut p.store, abi::BRIDGE_DEPTH_SET_EXPORT)
        .unwrap();
    for v in [7, 0, 499, -1] {
        set.call(&mut p.store, v).unwrap();
        assert_eq!(p.depth.get(&mut p.store).unwrap_i32(), v);
        assert_eq!(get.call(&mut p.store, ()).unwrap(), v);
    }
    p.depth.set(&mut p.store, Val::I32(3)).unwrap();
    assert_eq!(get.call(&mut p.store, ()).unwrap(), 3);
}

/// One instruction leaving a record of `op` at width `w` (RAX and RBX, or
/// BL), then `ret`.
fn snippet(op: FlagOp, w: Width) -> String {
    let prefix = match w {
        Width::W16 => "66",
        Width::W64 => "48",
        Width::W8 | Width::W32 => "",
    };
    let byte = w == Width::W8;
    let (opcode, rest) = match op {
        FlagOp::Add => (if byte { "00" } else { "01" }, "d8"),
        FlagOp::Adc => (if byte { "10" } else { "11" }, "d8"),
        FlagOp::Sbb => (if byte { "18" } else { "19" }, "d8"),
        FlagOp::Logic => (if byte { "20" } else { "21" }, "d8"),
        FlagOp::Sub => (if byte { "28" } else { "29" }, "d8"),
        FlagOp::Inc => (if byte { "fe" } else { "ff" }, "c0"),
        FlagOp::Dec => (if byte { "fe" } else { "ff" }, "c8"),
        FlagOp::Shl => (if byte { "c0" } else { "c1" }, "e003"),
        FlagOp::Shr => (if byte { "c0" } else { "c1" }, "e803"),
        FlagOp::Sar => (if byte { "c0" } else { "c1" }, "f803"),
        FlagOp::Mul => (if byte { "f6" } else { "f7" }, "e3"),
    };
    format!("{prefix}{opcode}{rest} c3")
}

/// `flags_materialize` gives what `run`'s step 3 gives, for every record
/// kind translated code leaves in the state block (each `FlagOp` at each
/// width, and materialized flags), then holds them materialized.
#[test]
fn flags_materialize_matches_run() {
    let widths = [Width::W8, Width::W16, Width::W32, Width::W64];
    let mut cases: Vec<(String, u32)> = vec![
        ("c3".into(), CC_EAGER),    // ret: the record run set up
        ("f9 c3".into(), CC_EAGER), // stc; ret
    ];
    for op in FlagOp::ALL {
        for w in widths {
            cases.push((snippet(op, w), op.cc_op(w)));
        }
    }
    let inputs: [(u64, u64, u64); 6] = [
        (0, 0, 0),
        (0x80, 0x80, abi::flags::CF),
        (u64::MAX, 1, 0),
        (0x7fff_ffff_ffff_ffff, 1, abi::flags::CF),
        (0x8000_0000_8000_8080, 0xffff_ffff_7fff_7f7f, 0),
        (0x1234_5678_9abc_def0, 0x0fed_cba9_8765_4321, abi::flags::CF),
    ];
    for (code, cc_op) in &cases {
        let (mut p, bridge) = with_bridge();
        link_run(&mut p, code, BASE);
        let materialize = bridge
            .get_typed_func::<i64, ()>(&mut p.store, abi::BRIDGE_FLAGS_MATERIALIZE_EXPORT)
            .unwrap();
        for &(a, b, cf) in &inputs {
            p.enter_state(BASE, &[(RAX, a), (RBX, b)]);
            // Status bits that `run` must replace, and others it keeps.
            let entry = 0x202 | abi::flags::DF | cf;
            p.write_u64(STATE + abi::STATE_RFLAGS, entry);
            assert_eq!(p.call_run(&bridge, abi::RUN_EXPORT), abi::EXIT_LEFT_CODE);
            assert_eq!(
                p.read_u32(STATE + abi::STATE_CC_OP),
                *cc_op,
                "{code}: record kind"
            );
            let by_run = p.read_u64(STATE + abi::STATE_RFLAGS);
            assert_eq!(by_run & !STATUS, entry & !STATUS, "{code}");
            // Every status bit flipped: flags_materialize must replace them.
            p.write_u64(STATE + abi::STATE_RFLAGS, by_run ^ STATUS);
            materialize.call(&mut p.store, STATE as i64).unwrap();
            let ctx = format!("{code} with rax {a:#x}, rbx {b:#x}, cf {cf}");
            assert_eq!(p.read_u64(STATE + abi::STATE_RFLAGS), by_run, "{ctx}");
            assert_eq!(p.read_u32(STATE + abi::STATE_CC_OP), CC_EAGER, "{ctx}");
            assert_eq!(p.read_u64(STATE + abi::STATE_CC_A), by_run, "{ctx}");
            // Materialized flags materialize to themselves.
            materialize.call(&mut p.store, STATE as i64).unwrap();
            assert_eq!(p.read_u64(STATE + abi::STATE_RFLAGS), by_run, "{ctx}");
        }
    }
}

/// The function at `TOMBSTONE_INDEX` stores `rip = LOOKUP_TOMBSTONE` and
/// `exit = EXIT_UNSUPPORTED`, changes nothing else, and returns STATUS_EXIT.
#[test]
fn tombstone_function_stops() {
    let (mut p, _bridge) = with_bridge();
    let pattern: Vec<u8> = (0..abi::STATE_SIZE).map(|i| (i * 5 + 1) as u8).collect();
    p.write(STATE, &pattern);
    let f = table_func(&mut p, abi::TOMBSTONE_INDEX)
        .typed::<(i64, i32, i64), i32>(&p.store)
        .unwrap();
    let status = f
        .call(&mut p.store, (STATE as i64, 0, abi::NO_RET as i64))
        .unwrap();
    assert_eq!(status, abi::STATUS_EXIT);
    let mut want = pattern;
    want[abi::STATE_RIP as usize..][..8].copy_from_slice(&abi::LOOKUP_TOMBSTONE.to_le_bytes());
    want[abi::STATE_EXIT as usize..][..4].copy_from_slice(&abi::EXIT_UNSUPPORTED.to_le_bytes());
    assert_eq!(p.read(STATE, abi::STATE_SIZE as usize), want);
}

/// The first key from `from` up whose probe starts at slot `h`.
fn key_at(h: u64, from: u64) -> u64 {
    (from..).find(|&k| abi::lookup_hash(k, MASK) == h).unwrap()
}

/// A process where a removed key's tombstone sits on the probe path of
/// both a live block (`target`, `mov ebx, 5; ret`) and `LOOKUP_TOMBSTONE`
/// itself, before the target's slot. Module A at `BASE` is `jmp rax`, and
/// its `run` is at `RUN_INDEX`. Returns the target.
fn tombstone_before_target() -> (Process, Instance, u64) {
    let (mut p, bridge) = with_bridge();
    link_run(&mut p, "ffe0", BASE);
    let h = abi::lookup_hash(abi::LOOKUP_TOMBSTONE, MASK);
    let target = key_at(h, 0x3_0000);
    // A key inserted before the target, then removed.
    let removed = key_at(h, 0x5000_0000);
    p.insert(removed, abi::TABLE_RESERVED, 0);
    let code = hex("bb05000000 c3");
    p.write(target, &code);
    p.link(&translate(&code, target, &Options::default()));
    assert_eq!(p.remove(removed), h, "the tombstone starts the path");
    assert_eq!(
        p.slot_of(target),
        Some((h + 1) & MASK),
        "the target follows it"
    );
    (p, bridge, target)
}

/// Lookups pass over a tombstone to a live key further along: the
/// trampoline's probe and translated code's.
#[test]
fn probes_pass_over_tombstones() {
    let (mut p, bridge, target) = tombstone_before_target();
    // The trampoline's probe for state.rip.
    p.enter_state(target, &[]);
    assert_eq!(p.call_run(&bridge, abi::RUN_EXPORT), abi::EXIT_LEFT_CODE);
    let s = p.state();
    assert_eq!((s.rip, s.gpr[RBX]), (abi::SENTINEL_RET, 5));
    // jmp rax: translated code's probe.
    p.enter_state(BASE, &[(RAX, target)]);
    assert_eq!(p.call_run(&bridge, abi::RUN_EXPORT), abi::EXIT_LEFT_CODE);
    let s = p.state();
    assert_eq!((s.rip, s.gpr[RBX]), (abi::SENTINEL_RET, 5));
}

/// A transfer to `LOOKUP_TOMBSTONE` itself hits the tombstone and stops
/// with EXIT_UNSUPPORTED at that address.
#[test]
fn transfer_to_the_tombstone_key_stops() {
    let (mut p, bridge, _) = tombstone_before_target();
    p.enter_state(abi::LOOKUP_TOMBSTONE, &[]);
    assert_eq!(p.call_run(&bridge, abi::RUN_EXPORT), abi::EXIT_UNSUPPORTED);
    assert_eq!(p.state().rip, abi::LOOKUP_TOMBSTONE);
    // jmp rax
    p.enter_state(BASE, &[(RAX, abi::LOOKUP_TOMBSTONE)]);
    assert_eq!(p.call_run(&bridge, abi::RUN_EXPORT), abi::EXIT_UNSUPPORTED);
    let s = p.state();
    assert_eq!(
        (s.rip, s.exit),
        (abi::LOOKUP_TOMBSTONE, abi::EXIT_UNSUPPORTED)
    );
}
