//! A wasmtime host for translated modules (ABI v4, `docs/design/abi.md`):
//! one memory64 guest memory, the process-wide funcref table, the
//! `table_base`/`lookup`/`depth` globals, the lookup table in guest memory
//! (in pages added after the machine's memory), the `fp` namespace (the
//! soft-float helpers of `x64wasm-fp`, defined natively with the status
//! kept per store), and `run(state)`. Shared by the spike tests and the
//! oracle replay so both exercise the same ABI.

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

/// Per-store data: the status word of the last `fp` helper call, which
/// `fp.status()` returns.
#[derive(Default)]
pub struct HostState {
    fp_status: u32,
}

/// Defines the `fp` helpers (`abi.md`, "Floating-point helper module") as
/// host functions in `store`, in [`abi::FpFunc`] order.
fn fp_funcs(store: &mut Store<HostState>) -> Vec<(&'static str, Func)> {
    use x64wasm_fp as fp;
    let mut out = Vec::new();
    // `$f` returns (result, status); the status goes to the store.
    macro_rules! def {
        ($name:literal, |$($arg:ident: $t:ty),*| -> $r:ty { $f:expr }) => {
            out.push((
                $name,
                Func::wrap(
                    &mut *store,
                    |mut caller: Caller<'_, HostState>, $($arg: $t),*| -> $r {
                        let (r, status) = $f;
                        caller.data_mut().fp_status = status;
                        r as $r
                    },
                ),
            ));
        };
    }
    macro_rules! sse2 {
        ($($name:literal => $func:ident, $t:ty;)*) => {$(
            def!($name, |a: $t, b: $t, m: i32| -> $t { fp::$func(a as _, b as _, m as u32) });
        )*};
    }
    sse2! {
        "f32_add" => f32_add, i32;
        "f32_sub" => f32_sub, i32;
        "f32_mul" => f32_mul, i32;
        "f32_div" => f32_div, i32;
        "f32_min" => f32_min, i32;
        "f32_max" => f32_max, i32;
    }
    def!("f32_sqrt", |a: i32, m: i32| -> i32 {
        fp::f32_sqrt(a as u32, m as u32)
    });
    sse2! {
        "f64_add" => f64_add, i64;
        "f64_sub" => f64_sub, i64;
        "f64_mul" => f64_mul, i64;
        "f64_div" => f64_div, i64;
        "f64_min" => f64_min, i64;
        "f64_max" => f64_max, i64;
    }
    def!("f64_sqrt", |a: i64, m: i32| -> i64 {
        fp::f64_sqrt(a as u64, m as u32)
    });
    def!("f32_cmp", |a: i32, b: i32, p: i32, m: i32| -> i32 {
        fp::f32_cmp(a as u32, b as u32, p as u32, m as u32)
    });
    def!("f64_cmp", |a: i64, b: i64, p: i32, m: i32| -> i32 {
        fp::f64_cmp(a as u64, b as u64, p as u32, m as u32)
    });
    def!("f32_comi", |a: i32, b: i32, s: i32, m: i32| -> i32 {
        fp::f32_comi(a as u32, b as u32, s as u32, m as u32)
    });
    def!("f64_comi", |a: i64, b: i64, s: i32, m: i32| -> i32 {
        fp::f64_comi(a as u64, b as u64, s as u32, m as u32)
    });
    def!("f32_to_f64", |a: i32, m: i32| -> i64 {
        fp::f32_to_f64(a as u32, m as u32)
    });
    def!("f64_to_f32", |a: i64, m: i32| -> i32 {
        fp::f64_to_f32(a as u64, m as u32)
    });
    def!("i32_to_f32", |a: i32, m: i32| -> i32 {
        fp::i32_to_f32(a, m as u32)
    });
    def!("i64_to_f32", |a: i64, m: i32| -> i32 {
        fp::i64_to_f32(a, m as u32)
    });
    def!("i64_to_f64", |a: i64, m: i32| -> i64 {
        fp::i64_to_f64(a, m as u32)
    });
    def!("f32_to_i32", |a: i32, m: i32, t: i32| -> i32 {
        fp::f32_to_i32(a as u32, m as u32, t as u32)
    });
    def!("f64_to_i32", |a: i64, m: i32, t: i32| -> i32 {
        fp::f64_to_i32(a as u64, m as u32, t as u32)
    });
    def!("f32_to_i64", |a: i32, m: i32, t: i32| -> i64 {
        fp::f32_to_i64(a as u32, m as u32, t as u32)
    });
    def!("f64_to_i64", |a: i64, m: i32, t: i32| -> i64 {
        fp::f64_to_i64(a as u64, m as u32, t as u32)
    });
    macro_rules! x87_arith {
        ($($name:literal => $func:ident;)*) => {$(
            def!($name, |am: i64, ase: i32, bm: i64, bse: i32, bk: i32, cw: i32| -> i64 {
                fp::$func(am as u64, ase as u32, bm as u64, bse as u32, bk as u32, cw as u32)
            });
        )*};
    }
    x87_arith! {
        "f80_add" => f80_add;
        "f80_sub" => f80_sub;
        "f80_subr" => f80_subr;
        "f80_mul" => f80_mul;
        "f80_div" => f80_div;
        "f80_divr" => f80_divr;
    }
    def!("f80_sqrt", |am: i64, ase: i32, cw: i32| -> i64 {
        fp::f80_sqrt(am as u64, ase as u32, cw as u32)
    });
    def!("f80_round", |am: i64, ase: i32, cw: i32| -> i64 {
        fp::f80_round(am as u64, ase as u32, cw as u32)
    });
    macro_rules! x87_pair {
        ($($name:literal => $func:ident;)*) => {$(
            def!($name, |am: i64, ase: i32, bm: i64, bse: i32, cw: i32| -> i64 {
                fp::$func(am as u64, ase as u32, bm as u64, bse as u32, cw as u32)
            });
        )*};
    }
    x87_pair! {
        "f80_scale" => f80_scale;
        "f80_prem" => f80_prem;
        "f80_prem1" => f80_prem1;
    }
    def!("f80_load", |bm: i64, bk: i32, cw: i32| -> i64 {
        fp::f80_load(bm as u64, bk as u32, cw as u32)
    });
    def!("f80_store", |am: i64, ase: i32, k: i32, cw: i32| -> i64 {
        fp::f80_store(am as u64, ase as u32, k as u32, cw as u32)
    });
    def!("f80_cmp", |am: i64,
                     ase: i32,
                     bm: i64,
                     bse: i32,
                     bk: i32,
                     q: i32,
                     cw: i32|
     -> i32 {
        fp::f80_cmp(
            am as u64, ase as u32, bm as u64, bse as u32, bk as u32, q as u32, cw as u32,
        )
    });
    out.push((
        "status",
        Func::wrap(&mut *store, |caller: Caller<'_, HostState>| -> i32 {
            caller.data().fp_status as i32
        }),
    ));
    out
}

impl Host {
    pub fn new() -> Result<Self> {
        Self::with_config(Config::new())
    }

    /// A host that compiles faster and runs slower: Cranelift without
    /// optimizations and with its single-pass register allocator. For many
    /// small modules that each run once, such as the L1 replay's (about
    /// half the compile time). Execution is the same: WebAssembly semantics
    /// don't depend on the compiler settings.
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
        let mut store = Store::new(&self.engine, HostState::default());
        let fp = fp_funcs(&mut store);
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
            fp,
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
    store: Store<HostState>,
    memory: Memory,
    table: Table,
    lookup_global: Global,
    depth: Global,
    /// Guest address of the lookup header.
    lookup: u64,
    /// The `fp` namespace (v4).
    fp: Vec<(&'static str, Func)>,
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
        for &(name, func) in &self.fp {
            linker.define(&self.store, abi::FP_MODULE, name, func)?;
        }
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
            move |mut caller: Caller<'_, HostState>,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The host defines every `fp` export with the signature of the
    /// contract, in its order.
    #[test]
    fn fp_namespace_matches_the_abi() {
        let host = Host::new().unwrap();
        let mut store = Store::new(&host.engine, HostState::default());
        let funcs = fp_funcs(&mut store);
        let names: Vec<&str> = funcs.iter().map(|&(n, _)| n).collect();
        let want: Vec<&str> = abi::FpFunc::ALL.iter().map(|f| f.name()).collect();
        assert_eq!(names, want);
        let val = |v: &abi::FpVal| match v {
            abi::FpVal::I32 => "i32",
            abi::FpVal::I64 => "i64",
        };
        let ty = |t: ValType| match t {
            ValType::I32 => "i32",
            ValType::I64 => "i64",
            _ => "other",
        };
        for (&(name, func), want) in funcs.iter().zip(abi::FpFunc::ALL) {
            let t = func.ty(&store);
            let params: Vec<&str> = t.params().map(ty).collect();
            let results: Vec<&str> = t.results().map(ty).collect();
            let want_params: Vec<&str> = want.params().iter().map(val).collect();
            let want_results: Vec<&str> = want.results().iter().map(val).collect();
            assert_eq!((params, results), (want_params, want_results), "{name}");
        }
    }
}
