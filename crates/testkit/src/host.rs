//! A wasmtime host for translated modules: one memory64 guest memory, a CPU
//! state block in it, and `run(state)`. Shared by the spike tests and the
//! oracle replay so both exercise the same ABI (`docs/design/abi.md`).

use anyhow::{Context, Result, ensure};
use wasmtime::{Config, Engine, Instance, Linker, Memory, MemoryType, Module, Store};
use x64wasm::abi::{self, CpuState};

const PAGE: u64 = 64 * 1024;

/// A wasmtime engine configured for translated modules. Compile once, run many.
pub struct Host {
    engine: Engine,
}

impl Host {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.wasm_memory64(true);
        config.wasm_threads(true);
        // ABI v2: tail transfers between translated functions use
        // `return_call` and `return_call_indirect`.
        config.wasm_tail_call(true);
        Ok(Host {
            engine: Engine::new(&config)?,
        })
    }

    /// Compiles a translated module.
    pub fn compile(&self, wasm: &[u8]) -> Result<Module> {
        Module::new(&self.engine, wasm)
            .map_err(anyhow::Error::from)
            .context("compiling translated module")
    }

    /// Creates a guest machine with `memory_bytes` of zeroed guest memory.
    pub fn machine(&self, memory_bytes: u64) -> Result<Machine> {
        let mut store = Store::new(&self.engine, ());
        let pages = memory_bytes.div_ceil(PAGE).max(1);
        let memory = Memory::new(&mut store, MemoryType::new64(pages, None))?;
        Ok(Machine {
            engine: self.engine.clone(),
            store,
            memory,
            instances: Vec::new(),
        })
    }
}

/// One guest memory plus the store that owns it.
pub struct Machine {
    engine: Engine,
    store: Store<()>,
    memory: Memory,
    /// Instances of modules already run on this machine. Translated modules
    /// keep no state of their own, so an instance can be reused; this keeps
    /// long test loops under the store's instance limit.
    instances: Vec<(Module, Instance)>,
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

    pub fn write_state(&mut self, state_addr: u64, state: &CpuState) -> Result<()> {
        self.write(state_addr, &state.to_bytes())
    }

    pub fn read_state(&self, state_addr: u64) -> Result<CpuState> {
        Ok(CpuState::from_bytes(
            &self.read(state_addr, abi::STATE_SIZE as usize)?,
        ))
    }

    /// Instantiates `module` against this machine's memory (once per module
    /// and machine) and calls `run(state_addr)`. Returns the exit reason.
    pub fn run(&mut self, module: &Module, state_addr: u64) -> Result<u32> {
        ensure!(
            Engine::same(module.engine(), &self.engine),
            "module was compiled by another engine"
        );
        let instance = match self.instances.iter().find(|(m, _)| Module::same(m, module)) {
            Some((_, instance)) => *instance,
            None => {
                let mut linker = Linker::new(&self.engine);
                linker.define(
                    &self.store,
                    abi::MEMORY_MODULE,
                    abi::MEMORY_NAME,
                    self.memory,
                )?;
                let instance: Instance = linker.instantiate(&mut self.store, module)?;
                self.instances.push((module.clone(), instance));
                instance
            }
        };
        let run = instance.get_typed_func::<i64, i32>(&mut self.store, abi::RUN_EXPORT)?;
        let exit = run.call(&mut self.store, state_addr as i64)?;
        Ok(exit as u32)
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

fn checked_range(size: usize, addr: u64, len: usize) -> Result<std::ops::Range<usize>> {
    let start = usize::try_from(addr)?;
    let end = start.checked_add(len).context("address overflow")?;
    ensure!(
        end <= size,
        "guest access {addr:#x}+{len:#x} is outside memory of {size:#x} bytes"
    );
    Ok(start..end)
}
