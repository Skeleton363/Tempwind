//! `cpuid` (`instruction-scope.md`, "`cpuid` results"): every row of the
//! table, zeros for other leaves, ECX ignored, the upper half of RAX
//! ignored, RFLAGS and the other registers unchanged. Runs the translated
//! instruction in wasmtime with a minimal ABI host (one module, no `fp`
//! imports). `cpuid` stays out of the L1 fixtures: a real CPU answers
//! differently.

use wasmtime::{
    Config, Engine, Global, GlobalType, Linker, Memory, MemoryType, Module, Mutability, Ref,
    RefType, Store, Table, TableType, Val, ValType,
};
use x64wasm::abi::{self, CpuState};
use x64wasm::{CodeRegion, Options};

const CODE: u64 = 0x1000;
const STATE: u64 = 0x2000;
const STACK: u64 = 0x8000;
const LOOKUP: u64 = 0x1_0000;
const SLOTS: u64 = LOOKUP + 0x40;
const MASK: u64 = 15;

/// The expected EAX, EBX, ECX, EDX of a leaf, written out from the table in
/// `instruction-scope.md`.
fn expected(leaf: u32) -> [u32; 4] {
    let brand = |part: usize| -> [u32; 4] {
        let mut s = *b"Tempwind virtual x86-64 CPU\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        s.rotate_left(16 * part);
        [0, 1, 2, 3].map(|r| u32::from_le_bytes(s[4 * r..4 * r + 4].try_into().unwrap()))
    };
    match leaf {
        0 => [1, 0x706D_6554, 0x5550_4356, 0x646E_6977],
        1 => [0x600, 0, 0x2000, 0x0780_8101],
        0x8000_0000 => [0x8000_0004, 0, 0, 0],
        0x8000_0001 => [0, 0, 1, 0x2010_0800],
        0x8000_0002 => brand(0),
        0x8000_0003 => brand(1),
        0x8000_0004 => brand(2),
        _ => [0; 4],
    }
}

struct Host {
    store: Store<()>,
    memory: Memory,
    run: wasmtime::TypedFunc<i64, i32>,
}

impl Host {
    /// Translates and links `cpuid; ret` at `CODE`.
    fn new() -> Host {
        let code = [0x0f, 0xa2, 0xc3];
        let t = x64wasm::translate(
            CodeRegion {
                base: CODE,
                bytes: &code,
            },
            &[CODE],
            &Options::default(),
        )
        .unwrap();
        assert!(t.unsupported.is_empty(), "cpuid is unsupported");
        let mut config = Config::new();
        config.wasm_memory64(true);
        config.wasm_tail_call(true);
        let engine = Engine::new(&config).unwrap();
        let module = Module::new(&engine, &t.wasm).unwrap();
        let blocks = x64wasm::link::read_blocks(&t.wasm).unwrap();
        let mut store = Store::new(&engine, ());
        let memory = Memory::new(&mut store, MemoryType::new64(2, None)).unwrap();
        let table = Table::new(
            &mut store,
            TableType::new(RefType::FUNCREF, blocks.chunks, None),
            Ref::Func(None),
        )
        .unwrap();
        let global = |store: &mut Store<()>, ty, v: Val, m| {
            Global::new(store, GlobalType::new(ty, m), v).unwrap()
        };
        let table_base = global(&mut store, ValType::I32, Val::I32(0), Mutability::Const);
        let lookup = global(
            &mut store,
            ValType::I64,
            Val::I64(LOOKUP as i64),
            Mutability::Const,
        );
        let depth = global(&mut store, ValType::I32, Val::I32(0), Mutability::Var);
        let mut linker = Linker::new(&engine);
        let env = abi::MEMORY_MODULE;
        linker
            .define(&store, env, abi::MEMORY_NAME, memory)
            .unwrap();
        linker.define(&store, env, abi::TABLE_NAME, table).unwrap();
        linker
            .define(&store, env, abi::TABLE_BASE_NAME, table_base)
            .unwrap();
        linker
            .define(&store, env, abi::LOOKUP_NAME, lookup)
            .unwrap();
        linker.define(&store, env, abi::DEPTH_NAME, depth).unwrap();
        let instance = linker.instantiate(&mut store, &module).unwrap();
        let run = instance
            .get_typed_func::<i64, i32>(&mut store, abi::RUN_EXPORT)
            .unwrap();
        // The lookup table (abi.md, "Lookup table").
        let data = memory.data_mut(&mut store);
        let put = |data: &mut [u8], at: u64, bytes: &[u8]| {
            data[at as usize..at as usize + bytes.len()].copy_from_slice(bytes);
        };
        put(data, LOOKUP + abi::LOOKUP_SLOTS, &SLOTS.to_le_bytes());
        put(data, LOOKUP + abi::LOOKUP_MASK, &MASK.to_le_bytes());
        put(
            data,
            LOOKUP + abi::LOOKUP_USED,
            &(blocks.blocks.len() as u64).to_le_bytes(),
        );
        for b in &blocks.blocks {
            let mut i = abi::lookup_hash(b.addr, MASK);
            let slot = |i: u64| SLOTS + i * abi::LOOKUP_SLOT_SIZE;
            while data[slot(i) as usize..][..8] != [0; 8] {
                i = (i + 1) & MASK;
            }
            put(data, slot(i), &b.addr.to_le_bytes());
            put(data, slot(i) + 8, &b.chunk.to_le_bytes());
            put(data, slot(i) + 12, &b.entry.to_le_bytes());
        }
        put(data, STACK, &abi::SENTINEL_RET.to_le_bytes());
        Host { store, memory, run }
    }

    fn run(&mut self, state: &CpuState) -> CpuState {
        let bytes = state.to_bytes();
        let data = self.memory.data_mut(&mut self.store);
        data[STATE as usize..][..bytes.len()].copy_from_slice(&bytes);
        let exit = self.run.call(&mut self.store, STATE as i64).unwrap();
        assert_eq!(exit as u32, abi::EXIT_LEFT_CODE);
        let data = self.memory.data(&self.store);
        CpuState::from_bytes(&data[STATE as usize..][..abi::STATE_SIZE as usize])
    }
}

#[test]
fn cpuid_returns_the_profile() {
    let mut host = Host::new();
    let leaves = [
        0,
        1,
        2,
        4,
        7,
        0xd,
        0x8000_0000,
        0x8000_0001,
        0x8000_0002,
        0x8000_0003,
        0x8000_0004,
        0x8000_0005,
        0x8000_0008,
        0x4000_0000,
        0xffff_ffff,
    ];
    for leaf in leaves {
        // Garbage in RAX's upper half and in ECX (the subleaf), every other
        // register distinct, and all status flags set or clear.
        for (subleaf, rflags) in [(0u64, 0x202u64), (0x1234_5678_0000_0001, 0x8d7)] {
            let mut input = CpuState {
                rip: CODE,
                rflags,
                ..CpuState::default()
            };
            for (i, r) in input.gpr.iter_mut().enumerate() {
                *r = 0x0101_0101_0101_0101u64.wrapping_mul(i as u64 + 1) ^ 0x8000_0000_0000_0000;
            }
            input.gpr[0] = 0xdead_beef_0000_0000 | leaf as u64;
            input.gpr[1] = subleaf;
            input.gpr[4] = STACK;
            let out = host.run(&input);
            let want = expected(leaf);
            let mut gpr = input.gpr;
            // EAX, EBX, ECX, EDX zero-extended into RAX, RBX, RCX, RDX.
            for (reg, v) in [0, 3, 1, 2].into_iter().zip(want) {
                gpr[reg] = v as u64;
            }
            gpr[4] = STACK + 8;
            assert_eq!(out.gpr, gpr, "leaf {leaf:#x}");
            assert_eq!(out.rflags, rflags, "leaf {leaf:#x}: RFLAGS changed");
            assert_eq!(out.rip, abi::SENTINEL_RET);
        }
    }
}

#[test]
fn vendor_and_brand_strings() {
    let mut host = Host::new();
    let mut regs = |leaf: u32| {
        let mut input = CpuState {
            rip: CODE,
            rflags: 0x202,
            ..CpuState::default()
        };
        input.gpr[0] = leaf as u64;
        input.gpr[4] = STACK;
        let out = host.run(&input);
        [0, 3, 1, 2].map(|r| out.gpr[r] as u32)
    };
    let vendor = regs(0);
    let text: Vec<u8> = [vendor[1], vendor[3], vendor[2]]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    assert_eq!(text, b"TempwindVCPU");
    let brand: Vec<u8> = (0x8000_0002..=0x8000_0004)
        .flat_map(&mut regs)
        .flat_map(|v| v.to_le_bytes())
        .collect();
    assert_eq!(brand.len(), 48);
    assert_eq!(
        brand.split(|&b| b == 0).next().unwrap(),
        b"Tempwind virtual x86-64 CPU"
    );
    assert!(brand[27..].iter().all(|&b| b == 0));
}
