//! A wasmtime host for translated modules (ABI v3, `docs/design/abi.md`):
//! one memory64 guest memory, the process-wide funcref table, the
//! `table_base`/`lookup`/`depth` globals, the lookup table in guest memory
//! (in pages added after the machine's memory), and `run(state)`. Shared by
//! the spike tests and the oracle replay so both exercise the same ABI.

use std::sync::Arc;

use anyhow::{Context, Result, bail, ensure};
use wasmtime::{
    Caller, Config, Engine, Func, Global, GlobalType, Instance, Linker, Memory, MemoryType,
    Mutability, Ref, RefType, Store, Table, TableType, Val, ValType,
};
use x64wasm::abi::{self, CpuState};
use x64wasm::link::ModuleBlocks;

const PAGE: u64 = 64 * 1024;
/// Slots of a fresh lookup table (a power of two).
const INITIAL_SLOTS: u64 = 256;
/// Offset of the first slot array from the lookup header.
const SLOTS_OFFSET: u64 = 64;

/// A wasmtime engine configured for translated modules. Compile once, run many.
pub struct Host {
    engine: Engine,
}

/// A compiled translated module and its `x64wasm.blocks` section.
#[derive(Clone)]
pub struct Module {
    pub module: wasmtime::Module,
    pub blocks: Arc<ModuleBlocks>,
}

impl Module {
    fn same(&self, other: &Module) -> bool {
        wasmtime::Module::same(&self.module, &other.module)
    }
}

impl Host {
    pub fn new() -> Result<Self> {
        Self::with_config(Config::new())
    }

    /// A host that compiles fast and runs slower: Cranelift without
    /// optimizations, with its single-pass register allocator. For many
    /// small modules each run once, such as the L1 replay's. The semantics
    /// are the same (WebAssembly execution is deterministic).
    pub fn fast_compile() -> Result<Self> {
        let mut config = Config::new();
        config.cranelift_opt_level(wasmtime::OptLevel::None);
        config.cranelift_regalloc_algorithm(wasmtime::RegallocAlgorithm::SinglePass);
        Self::with_config(config)
    }

    fn with_config(mut config: Config) -> Result<Self> {
        config.wasm_memory64(true);
        config.wasm_threads(true);
        // Transfers between translated functions use `return_call` and
        // `return_call_indirect`.
        config.wasm_tail_call(true);
        Ok(Host {
            engine: Engine::new(&config)?,
        })
    }

    /// Compiles a translated module and reads its blocks.
    pub fn compile(&self, wasm: &[u8]) -> Result<Module> {
        let blocks = x64wasm::link::read_blocks(wasm)?;
        let module = wasmtime::Module::new(&self.engine, wasm)
            .map_err(anyhow::Error::from)
            .context("compiling translated module")?;
        Ok(Module {
            module,
            blocks: Arc::new(blocks),
        })
    }

    /// Creates a guest machine with `memory_bytes` of zeroed guest memory,
    /// followed by the pages that hold the lookup table.
    pub fn machine(&self, memory_bytes: u64) -> Result<Machine> {
        let mut store = Store::new(&self.engine, ());
        let pages = memory_bytes.div_ceil(PAGE).max(1);
        let lookup = pages * PAGE;
        let memory = Memory::new(&mut store, MemoryType::new64(pages + 1, None))?;
        let table = Table::new(
            &mut store,
            TableType::new(RefType::FUNCREF, 0, None),
            Ref::Func(None),
        )?;
        let lookup_global = Global::new(
            &mut store,
            GlobalType::new(ValType::I64, Mutability::Const),
            Val::I64(lookup as i64),
        )?;
        let depth = Global::new(
            &mut store,
            GlobalType::new(ValType::I32, Mutability::Var),
            Val::I32(0),
        )?;
        let mut machine = Machine {
            engine: self.engine.clone(),
            store,
            memory,
            table,
            lookup_global,
            depth,
            lookup,
            next_alloc: lookup + SLOTS_OFFSET,
            instances: Vec::new(),
            linked: Vec::new(),
        };
        machine.unlink_all()?;
        Ok(machine)
    }
}

struct Linked {
    module: Module,
    instance: Instance,
    table_base: u32,
}

/// One guest memory, its table, globals and lookup table, plus the store
/// that owns them: a process of linked modules.
pub struct Machine {
    engine: Engine,
    store: Store<()>,
    memory: Memory,
    table: Table,
    lookup_global: Global,
    depth: Global,
    /// Guest address of the lookup header.
    lookup: u64,
    /// Where the next slot array goes.
    next_alloc: u64,
    /// Instances of modules used on this machine. Each has its own table
    /// slots for good, so it can be relinked without instantiating again;
    /// this keeps long test loops under the store's instance limit.
    instances: Vec<Linked>,
    /// Indices into `instances` whose blocks are in the lookup table.
    linked: Vec<usize>,
}

impl Machine {
    /// Copies `bytes` into guest memory at `addr`.
    pub fn write(&mut self, addr: u64, bytes: &[u8]) -> Result<()> {
        let data = self.memory.data_mut(&mut self.store);
        let range = checked_range(data.len(), addr, bytes.len())?;
        data[range].copy_from_slice(bytes);
        Ok(())
    }

    /// Reads `len` bytes of guest memory at `addr`.
    pub fn read(&self, addr: u64, len: usize) -> Result<Vec<u8>> {
        let data = self.memory.data(&self.store);
        let range = checked_range(data.len(), addr, len)?;
        Ok(data[range].to_vec())
    }

    fn read_u64(&self, addr: u64) -> Result<u64> {
        let b = self.read(addr, 8)?;
        Ok(u64::from_le_bytes(
            b.try_into().map_err(|_| anyhow::anyhow!("short read"))?,
        ))
    }

    fn write_u64(&mut self, addr: u64, v: u64) -> Result<()> {
        self.write(addr, &v.to_le_bytes())
    }

    pub fn write_state(&mut self, state_addr: u64, state: &CpuState) -> Result<()> {
        self.write(state_addr, &state.to_bytes())
    }

    pub fn read_state(&self, state_addr: u64) -> Result<CpuState> {
        Ok(CpuState::from_bytes(
            &self.read(state_addr, abi::STATE_SIZE as usize)?,
        ))
    }

    /// Number of slots of the lookup table.
    pub fn lookup_slots(&self) -> usize {
        self.read_u64(self.lookup + abi::LOOKUP_MASK)
            .map_or(0, |m| m as usize + 1)
    }

    /// A zeroed slot array of `slots` slots, in pages added as needed.
    fn alloc_slots(&mut self, slots: u64) -> Result<u64> {
        let addr = self.next_alloc.next_multiple_of(16);
        let bytes = slots * abi::LOOKUP_SLOT_SIZE;
        let end = addr + bytes;
        let size = self.memory.data_size(&self.store) as u64;
        if end > size {
            self.memory
                .grow(&mut self.store, (end - size).div_ceil(PAGE))?;
        }
        self.write(addr, &vec![0u8; bytes as usize])?;
        self.next_alloc = end;
        Ok(addr)
    }

    /// Empties the lookup table: no module is linked any more (their table
    /// slots stay).
    pub fn unlink_all(&mut self) -> Result<()> {
        self.next_alloc = self.lookup + SLOTS_OFFSET;
        let slots = self.alloc_slots(INITIAL_SLOTS)?;
        self.write(self.lookup, &[0u8; abi::LOOKUP_HEADER_SIZE as usize])?;
        self.write_u64(self.lookup + abi::LOOKUP_SLOTS, slots)?;
        self.write_u64(self.lookup + abi::LOOKUP_MASK, INITIAL_SLOTS - 1)?;
        self.linked.clear();
        Ok(())
    }

    /// Inserts a lookup slot (abi.md, "Lookup table"); a key that is already
    /// there keeps its slot. Grows the table to keep it at most half full.
    pub fn insert(&mut self, key: u64, func: u32, entry: u32) -> Result<()> {
        ensure!(key != 0, "address 0 can't be a lookup key");
        let used = self.read_u64(self.lookup + abi::LOOKUP_USED)?;
        let mask = self.read_u64(self.lookup + abi::LOOKUP_MASK)?;
        if 2 * (used + 1) > mask + 1 {
            self.rehash(2 * (mask + 1))?;
        }
        let slots = self.read_u64(self.lookup + abi::LOOKUP_SLOTS)?;
        let mask = self.read_u64(self.lookup + abi::LOOKUP_MASK)?;
        let mut i = abi::lookup_hash(key, mask);
        loop {
            let at = slots + i * abi::LOOKUP_SLOT_SIZE;
            match self.read_u64(at)? {
                0 => break self.write_slot(at, key, func, entry)?,
                k if k == key => return Ok(()),
                _ => i = (i + 1) & mask,
            }
        }
        self.write_u64(self.lookup + abi::LOOKUP_USED, used + 1)
    }

    fn write_slot(&mut self, at: u64, key: u64, func: u32, entry: u32) -> Result<()> {
        let mut slot = [0u8; 16];
        slot[..8].copy_from_slice(&key.to_le_bytes());
        slot[8..12].copy_from_slice(&func.to_le_bytes());
        slot[12..].copy_from_slice(&entry.to_le_bytes());
        self.write(at, &slot)
    }

    /// Moves every slot into a new array of `n` slots.
    fn rehash(&mut self, n: u64) -> Result<()> {
        let old = self.read_u64(self.lookup + abi::LOOKUP_SLOTS)?;
        let old_n = self.read_u64(self.lookup + abi::LOOKUP_MASK)? + 1;
        let old_bytes = self.read(old, (old_n * abi::LOOKUP_SLOT_SIZE) as usize)?;
        let new = self.alloc_slots(n)?;
        let mask = n - 1;
        for slot in old_bytes.chunks_exact(16) {
            let key = u64::from_le_bytes(slot[..8].try_into()?);
            if key == 0 {
                continue;
            }
            let mut i = abi::lookup_hash(key, mask);
            while self.read_u64(new + i * abi::LOOKUP_SLOT_SIZE)? != 0 {
                i = (i + 1) & mask;
            }
            self.write(new + i * abi::LOOKUP_SLOT_SIZE, slot)?;
        }
        self.write_u64(self.lookup + abi::LOOKUP_SLOTS, new)?;
        self.write_u64(self.lookup + abi::LOOKUP_MASK, mask)
    }

    /// Table slots `[base, base + n)`, growing the table.
    fn alloc_table(&mut self, n: u32) -> Result<u32> {
        let base = self.table.size(&self.store);
        self.table
            .grow(&mut self.store, n.into(), Ref::Func(None))?;
        u32::try_from(base).context("table too large")
    }

    /// The instance of `module` on this machine, instantiated on first use.
    fn instance(&mut self, module: &Module) -> Result<usize> {
        ensure!(
            Engine::same(module.module.engine(), &self.engine),
            "module was compiled by another engine"
        );
        if let Some(i) = self.instances.iter().position(|l| l.module.same(module)) {
            return Ok(i);
        }
        let table_base = self.alloc_table(module.blocks.chunks)?;
        let base_global = Global::new(
            &mut self.store,
            GlobalType::new(ValType::I32, Mutability::Const),
            Val::I32(table_base as i32),
        )?;
        let mut linker = Linker::new(&self.engine);
        let env = abi::MEMORY_MODULE;
        linker.define(&self.store, env, abi::MEMORY_NAME, self.memory)?;
        linker.define(&self.store, env, abi::TABLE_NAME, self.table)?;
        linker.define(&self.store, env, abi::TABLE_BASE_NAME, base_global)?;
        linker.define(&self.store, env, abi::LOOKUP_NAME, self.lookup_global)?;
        linker.define(&self.store, env, abi::DEPTH_NAME, self.depth)?;
        let instance = linker.instantiate(&mut self.store, &module.module)?;
        self.instances.push(Linked {
            module: module.clone(),
            instance,
            table_base,
        });
        Ok(self.instances.len() - 1)
    }

    /// Links `module` into this machine's process (abi.md, "Linking a
    /// module"): instantiates it (once per machine) and inserts its blocks,
    /// keeping slots that already exist. Returns its `table_base`.
    pub fn link(&mut self, module: &Module) -> Result<u32> {
        let i = self.instance(module)?;
        let table_base = self.instances[i].table_base;
        if !self.linked.contains(&i) {
            let blocks = self.instances[i].module.blocks.clone();
            for b in &blocks.blocks {
                self.insert(b.addr, table_base + b.chunk, b.entry)?;
            }
            self.linked.push(i);
        }
        Ok(table_base)
    }

    /// Adds a host function (abi.md, "Host functions") to the table and
    /// returns its table index. Map guest addresses to it with
    /// [`Machine::insert`].
    pub fn add_host_function(
        &mut self,
        f: impl Fn(&mut HostCall<'_>) -> Result<i32> + Send + Sync + 'static,
    ) -> Result<u32> {
        let memory = self.memory;
        let func = Func::wrap(
            &mut self.store,
            move |mut caller: Caller<'_, ()>,
                  state: i64,
                  entry: i32,
                  ret: i64|
                  -> wasmtime::Result<i32> {
                let mut call = HostCall {
                    mem: memory.data_mut(&mut caller),
                    state: state as u64,
                    entry: entry as u32,
                    ret: ret as u64,
                };
                f(&mut call).map_err(wasmtime::Error::from_anyhow)
            },
        );
        let index = self.alloc_table(1)?;
        self.table
            .set(&mut self.store, index.into(), Ref::Func(Some(func)))?;
        Ok(index)
    }

    /// Calls `run(state_addr)` of `module`, which must be linked, without
    /// changing what is linked. Returns the exit reason.
    pub fn enter(&mut self, module: &Module, state_addr: u64) -> Result<u32> {
        let Some(&i) = self
            .linked
            .iter()
            .find(|&&i| self.instances[i].module.same(module))
        else {
            bail!("module is not linked");
        };
        let instance = self.instances[i].instance;
        let run = instance.get_typed_func::<i64, i32>(&mut self.store, abi::RUN_EXPORT)?;
        let exit = run.call(&mut self.store, state_addr as i64)?;
        Ok(exit as u32)
    }

    /// Runs `module` alone: the lookup table holds exactly its blocks
    /// (relinked if other modules were linked), then `run(state_addr)`.
    /// Returns the exit reason.
    pub fn run(&mut self, module: &Module, state_addr: u64) -> Result<u32> {
        let i = self.instance(module)?;
        if self.linked != [i] {
            self.unlink_all()?;
            self.link(module)?;
        }
        self.enter(module, state_addr)
    }

    /// Writes `state`, runs, and returns the resulting state.
    pub fn run_state(
        &mut self,
        module: &Module,
        state_addr: u64,
        state: &CpuState,
    ) -> Result<CpuState> {
        self.write_state(state_addr, state)?;
        let exit = self.run(module, state_addr)?;
        let out = self.read_state(state_addr)?;
        ensure!(
            out.exit == exit,
            "run returned {exit} but the state block says {}",
            out.exit
        );
        Ok(out)
    }
}

/// A call of a host function: its arguments and guest memory.
pub struct HostCall<'a> {
    mem: &'a mut [u8],
    /// Guest address of the state block.
    pub state: u64,
    /// The entry of the lookup slot that named the function.
    pub entry: u32,
    /// The return address the wasm caller expects.
    pub ret: u64,
}

impl HostCall<'_> {
    pub fn read_u64(&self, addr: u64) -> Result<u64> {
        let range = checked_range(self.mem.len(), addr, 8)?;
        Ok(u64::from_le_bytes(self.mem[range].try_into()?))
    }

    pub fn write_u64(&mut self, addr: u64, v: u64) -> Result<()> {
        let range = checked_range(self.mem.len(), addr, 8)?;
        self.mem[range].copy_from_slice(&v.to_le_bytes());
        Ok(())
    }

    /// General-purpose register `i` from the state block (0 if out of range).
    pub fn gpr(&self, i: usize) -> u64 {
        self.read_u64(self.state + abi::gpr_offset(i as u32))
            .unwrap_or(0)
    }

    pub fn set_gpr(&mut self, i: usize, v: u64) {
        let _ = self.write_u64(self.state + abi::gpr_offset(i as u32), v);
    }

    /// Performs the guest's `ret`: pops `t = [RSP]`; returns
    /// `STATUS_RETURNED` if `t` is the expected return address, else stores
    /// `state.rip = t` and returns `STATUS_UNWIND`.
    pub fn ret(&mut self) -> Result<i32> {
        let rsp = self.gpr(4);
        let t = self.read_u64(rsp)?;
        self.set_gpr(4, rsp.wrapping_add(8));
        if t == self.ret {
            return Ok(abi::STATUS_RETURNED);
        }
        self.write_u64(self.state + abi::STATE_RIP, t)?;
        Ok(abi::STATUS_UNWIND)
    }

    /// Stops execution: `state.rip = rip`, `state.exit = EXIT_HOST`; returns
    /// `STATUS_EXIT`.
    pub fn stop(&mut self, rip: u64) -> Result<i32> {
        self.write_u64(self.state + abi::STATE_RIP, rip)?;
        let at = checked_range(self.mem.len(), self.state + abi::STATE_EXIT, 4)?;
        self.mem[at].copy_from_slice(&abi::EXIT_HOST.to_le_bytes());
        Ok(abi::STATUS_EXIT)
    }
}

fn checked_range(size: usize, addr: u64, len: usize) -> Result<std::ops::Range<usize>> {
    let start = usize::try_from(addr)?;
    let end = start.checked_add(len).context("address overflow")?;
    ensure!(
        end <= size,
        "guest access {addr:#x}+{len:#x} is outside memory of {size:#x} bytes"
    );
    Ok(start..end)
}
