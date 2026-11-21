//! A small Wine-process host for the bridge and store-check tests, in
//! wasmtime: one shared memory64 (`abi.md` v5, "Instances"), one funcref
//! table with the bridge's reserved slots, the `depth` global, a lookup
//! table and a page-state table. `testkit`'s host keeps a non-shared memory
//! and no reserved slots, so these tests carry their own.
#![allow(dead_code)]

use wasmtime::{
    Config, Engine, Global, GlobalType, Instance, Linker, MemoryType, Module, Mutability, Ref,
    RefType, SharedMemory, Store, Table, TableType, Val, ValType,
};
use x64wasm::abi::{self, CpuState};
use x64wasm::{CodeRegion, Options};

/// The guest region: `[0, GUEST_BYTES)`.
pub const GUEST_BYTES: u64 = 0x10_0000;
/// The page-state table of the guest region, one byte per 4 KiB page.
pub const PAGE_FLAGS: u64 = GUEST_BYTES;
/// The lookup header.
pub const LOOKUP: u64 = GUEST_BYTES + 0x1000;
/// The slot array, `MASK + 1` slots.
pub const SLOTS: u64 = GUEST_BYTES + 0x2000;
pub const MASK: u64 = 1023;
/// Where tests put the state block and the stack.
pub const STATE: u64 = 0x8_0000;
pub const STACK: u64 = 0xf_0000;
const MEMORY_PAGES: u64 = 32;
const _: () = assert!(SLOTS + (MASK + 1) * abi::LOOKUP_SLOT_SIZE <= MEMORY_PAGES << 16);

pub fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Translates `code` at `base` (one function entry at `base`) for a Wine
/// process: shared memory, plus `opts`.
pub fn translate(code: &[u8], base: u64, opts: &Options) -> Vec<u8> {
    let opts = Options {
        shared_memory: true,
        ..opts.clone()
    };
    x64wasm::translate(CodeRegion { base, bytes: code }, &[base], &opts)
        .unwrap()
        .wasm
}

pub struct Process {
    pub engine: Engine,
    pub store: Store<()>,
    pub memory: SharedMemory,
    pub table: Table,
    pub depth: Global,
    lookup: Global,
    page_flags: Global,
}

impl Process {
    pub fn new() -> Self {
        let mut config = Config::new();
        config.wasm_memory64(true);
        config.wasm_threads(true);
        config.shared_memory(true);
        config.wasm_tail_call(true);
        let engine = Engine::new(&config).unwrap();
        let mut store = Store::new(&engine, ());
        let ty = MemoryType::builder()
            .memory64(true)
            .shared(true)
            .min(MEMORY_PAGES)
            .max(Some(abi::MAX_MEMORY_PAGES))
            .build()
            .unwrap();
        let memory = SharedMemory::new(&engine, ty).unwrap();
        let table = Table::new(
            &mut store,
            TableType::new(RefType::FUNCREF, abi::TABLE_RESERVED, None),
            Ref::Func(None),
        )
        .unwrap();
        let global = |store: &mut Store<()>, v: Val, m: Mutability| {
            let ty = GlobalType::new(v.ty(&*store).unwrap(), m);
            Global::new(store, ty, v).unwrap()
        };
        let depth = global(&mut store, Val::I32(0), Mutability::Var);
        let lookup = global(&mut store, Val::I64(LOOKUP as i64), Mutability::Const);
        let page_flags = global(&mut store, Val::I64(PAGE_FLAGS as i64), Mutability::Const);
        let p = Process {
            engine,
            store,
            memory,
            table,
            depth,
            lookup,
            page_flags,
        };
        p.write_u64(LOOKUP + abi::LOOKUP_SLOTS, SLOTS);
        p.write_u64(LOOKUP + abi::LOOKUP_MASK, MASK);
        p
    }

    pub fn write(&self, addr: u64, bytes: &[u8]) {
        let data = self.memory.data();
        let range = &data[addr as usize..][..bytes.len()];
        for (cell, &b) in range.iter().zip(bytes) {
            // One thread: nothing else touches the memory.
            unsafe { *cell.get() = b };
        }
    }

    pub fn read(&self, addr: u64, len: usize) -> Vec<u8> {
        let data = self.memory.data();
        data[addr as usize..][..len]
            .iter()
            .map(|cell| unsafe { *cell.get() })
            .collect()
    }

    pub fn read_u64(&self, addr: u64) -> u64 {
        u64::from_le_bytes(self.read(addr, 8).try_into().unwrap())
    }

    pub fn read_u32(&self, addr: u64) -> u32 {
        u32::from_le_bytes(self.read(addr, 4).try_into().unwrap())
    }

    pub fn write_u64(&self, addr: u64, v: u64) {
        self.write(addr, &v.to_le_bytes());
    }

    pub fn write_u32(&self, addr: u64, v: u32) {
        self.write(addr, &v.to_le_bytes());
    }

    /// Sets the page-state byte of every page `[addr, addr + len)` touches.
    pub fn set_page_flags(&self, addr: u64, len: u64, flags: u8) {
        let shift = abi::PAGE_FLAGS_SHIFT;
        let n = ((addr + len - 1) >> shift) - (addr >> shift) + 1;
        self.write(PAGE_FLAGS + (addr >> shift), &vec![flags; n as usize]);
    }

    /// Writes a state block at [`STATE`] that returns to `SENTINEL_RET`
    /// from [`STACK`]: `rip`, then `regs` (by GPR number).
    pub fn enter_state(&self, rip: u64, regs: &[(usize, u64)]) {
        self.write(STACK, &abi::SENTINEL_RET.to_le_bytes());
        let mut state = CpuState {
            rip,
            rflags: 0x202,
            ..CpuState::default()
        };
        state.gpr[4] = STACK;
        for &(r, v) in regs {
            state.gpr[r] = v;
        }
        self.write(STATE, &state.to_bytes());
    }

    pub fn state(&self) -> CpuState {
        CpuState::from_bytes(&self.read(STATE, abi::STATE_SIZE as usize))
    }

    /// The slot index of `key`, if the table holds it.
    fn find(&self, key: u64) -> Option<u64> {
        let mut i = abi::lookup_hash(key, MASK);
        loop {
            match self.read_u64(SLOTS + i * abi::LOOKUP_SLOT_SIZE) {
                0 => return None,
                k if k == key => return Some(i),
                _ => i = (i + 1) & MASK,
            }
        }
    }

    /// Inserts a lookup slot at the end of `key`'s probe path; a key already
    /// there keeps its slot. (A full host reuses the first tombstone on the
    /// path; these tests place tombstones themselves.)
    pub fn insert(&self, key: u64, func: u32, entry: u32) {
        assert_ne!(key, 0);
        if self.find(key).is_some() {
            return;
        }
        let mut i = abi::lookup_hash(key, MASK);
        while self.read_u64(SLOTS + i * abi::LOOKUP_SLOT_SIZE) != 0 {
            i = (i + 1) & MASK;
        }
        let at = SLOTS + i * abi::LOOKUP_SLOT_SIZE;
        self.write_u64(at, key);
        self.write_u32(at + 8, func);
        self.write_u32(at + 12, entry);
        let used = self.read_u64(LOOKUP + abi::LOOKUP_USED);
        self.write_u64(LOOKUP + abi::LOOKUP_USED, used + 1);
        assert!(2 * (used + 1) <= MASK + 1);
    }

    /// Removes `key` (abi.md v5, "Lookup table"): its slot becomes a
    /// tombstone. Returns the slot index.
    pub fn remove(&self, key: u64) -> u64 {
        let i = self.find(key).expect("key to remove");
        let at = SLOTS + i * abi::LOOKUP_SLOT_SIZE;
        self.write_u64(at, abi::LOOKUP_TOMBSTONE);
        self.write_u32(at + 8, abi::TOMBSTONE_INDEX);
        self.write_u32(at + 12, 0);
        i
    }

    /// The slot index `key` lives at.
    pub fn slot_of(&self, key: u64) -> Option<u64> {
        self.find(key)
    }

    fn linker(&self) -> Linker<()> {
        let mut linker = Linker::new(&self.engine);
        let env = abi::MEMORY_MODULE;
        linker
            .define(&self.store, env, abi::MEMORY_NAME, self.memory.clone())
            .unwrap();
        linker
            .define(&self.store, env, abi::TABLE_NAME, self.table)
            .unwrap();
        linker
            .define(&self.store, env, abi::DEPTH_NAME, self.depth)
            .unwrap();
        linker
            .define(&self.store, env, abi::LOOKUP_NAME, self.lookup)
            .unwrap();
        linker
            .define(&self.store, env, abi::PAGE_FLAGS_NAME, self.page_flags)
            .unwrap();
        linker
    }

    /// Instantiates a module with the process's imports (and
    /// `env.table_base`, if given).
    pub fn instantiate(&mut self, wasm: &[u8], table_base: Option<u32>) -> Instance {
        let module = Module::new(&self.engine, wasm).unwrap();
        let mut linker = self.linker();
        if let Some(base) = table_base {
            let ty = GlobalType::new(ValType::I32, Mutability::Const);
            let g = Global::new(&mut self.store, ty, Val::I32(base as i32)).unwrap();
            linker
                .define(&self.store, abi::MEMORY_MODULE, abi::TABLE_BASE_NAME, g)
                .unwrap();
        }
        linker.instantiate(&mut self.store, &module).unwrap()
    }

    /// Links a translated module (abi.md, "Linking a module"): table slots
    /// past the ones in use, then its blocks into the lookup table.
    pub fn link(&mut self, wasm: &[u8]) -> Instance {
        let blocks = x64wasm::link::read_blocks(wasm).unwrap();
        let base = self.table.size(&self.store) as u32;
        self.table
            .grow(&mut self.store, blocks.chunks.into(), Ref::Func(None))
            .unwrap();
        let instance = self.instantiate(wasm, Some(base));
        for b in &blocks.blocks {
            self.insert(b.addr, base + b.chunk, b.entry);
        }
        instance
    }

    /// Calls the `(state i64) -> i32` export `name` of `instance` on [`STATE`].
    pub fn call_run(&mut self, instance: &Instance, name: &str) -> u32 {
        let run = instance
            .get_typed_func::<i64, i32>(&mut self.store, name)
            .unwrap();
        run.call(&mut self.store, STATE as i64).unwrap() as u32
    }
}
