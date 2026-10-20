//! Running guest machine code natively (x86-64 Unix hosts only), for
//! differential tests and native-vs-wasm timing.
//!
//! - [`ExecPage`] maps code read+execute anywhere and hands out a function
//!   pointer to it.
//! - [`Sandbox`] runs a snippet with a full guest register file (including
//!   RSP and the status flags) against a data page at a fixed address, so the
//!   same snippet and addresses can also be run through the translator and
//!   the results compared bit for bit.

use std::sync::Mutex;

use anyhow::{Result, bail, ensure};
use iced_x86::code_asm::*;
use x64wasm::abi::{self, CpuState};

/// Page size assumed for mappings.
const PAGE: usize = 4096;

/// Anonymous memory mapped with `prot`; unmapped on drop.
struct Mapping {
    ptr: *mut u8,
    len: usize,
}

// SAFETY: the mapping is plain memory owned by this value.
unsafe impl Send for Mapping {}

impl Mapping {
    fn new(fixed: Option<u64>, len: usize) -> Result<Mapping> {
        let len = len.div_ceil(PAGE).max(1) * PAGE;
        let (addr, flags) = match fixed {
            Some(a) => (a as *mut libc::c_void, libc::MAP_FIXED_NOREPLACE),
            None => (std::ptr::null_mut(), 0),
        };
        // SAFETY: an anonymous private mapping; with MAP_FIXED_NOREPLACE the
        // kernel refuses to replace an existing mapping.
        let ptr = unsafe {
            libc::mmap(
                addr,
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | flags,
                -1,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            bail!(
                "mmap({fixed:x?}, {len:#x}) failed: {}",
                std::io::Error::last_os_error()
            );
        }
        let mapping = Mapping {
            ptr: ptr as *mut u8,
            len,
        };
        if let Some(a) = fixed {
            ensure!(
                ptr as u64 == a,
                "mmap placed the fixed mapping at {ptr:p} instead of {a:#x}"
            );
        }
        Ok(mapping)
    }

    fn protect(&self, prot: libc::c_int) -> Result<()> {
        // SAFETY: changes protection of memory this value owns.
        let rc = unsafe { libc::mprotect(self.ptr as *mut libc::c_void, self.len, prot) };
        ensure!(
            rc == 0,
            "mprotect failed: {}",
            std::io::Error::last_os_error()
        );
        Ok(())
    }

    fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: the mapping is readable and writable while this borrow lasts
        // (callers only use this between protect(RW) calls).
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    fn bytes(&self) -> &[u8] {
        // SAFETY: the mapping is readable.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: unmaps memory this value owns.
        unsafe {
            libc::munmap(self.ptr as *mut libc::c_void, self.len);
        }
    }
}

/// Machine code mapped read+execute at an address the kernel picks.
pub struct ExecPage {
    map: Mapping,
}

impl ExecPage {
    pub fn new(code: &[u8]) -> Result<ExecPage> {
        let mut map = Mapping::new(None, code.len())?;
        map.bytes_mut()[..code.len()].copy_from_slice(code);
        map.protect(libc::PROT_READ | libc::PROT_EXEC)?;
        Ok(ExecPage { map })
    }

    /// Address of the first byte.
    pub fn addr(&self) -> u64 {
        self.map.ptr as u64
    }

    /// Calls the code as `extern "win64" fn(rcx) -> rax`.
    ///
    /// # Safety
    /// The code must be a well-behaved Win64 leaf-style function: it may only
    /// touch its own stack and must return with `ret`.
    pub unsafe fn call_win64(&self, arg: u64) -> u64 {
        // SAFETY: the page holds executable code; the caller vouches for it.
        let f: extern "win64" fn(u64) -> u64 = unsafe { std::mem::transmute(self.map.ptr) };
        f(arg)
    }
}

/// Guest data page of the sandbox (fixed address, same in guest memory).
pub const SANDBOX_DATA: u64 = 0x2000_0000;
/// Size of the data page.
pub const SANDBOX_DATA_SIZE: usize = 0x1_0000;
/// Native-only register block (abi layout plus the host RSP at 0x98).
const SANDBOX_STATE: u64 = 0x2010_0000;
const HOST_RSP: u64 = 0x98;
/// Code page; the snippet starts at [`SANDBOX_SNIPPET`].
const SANDBOX_CODE: u64 = 0x2020_0000;
const SANDBOX_CODE_SIZE: usize = 0x1_0000;
/// Where snippets are placed (and translated at).
pub const SANDBOX_SNIPPET: u64 = SANDBOX_CODE + 0x100;
/// Guest memory needed to mirror the sandbox's addresses.
pub const SANDBOX_MEMORY_BYTES: u64 = SANDBOX_CODE + SANDBOX_CODE_SIZE as u64;

/// Fixed-address native sandbox. Only one exists per process.
pub struct Sandbox {
    data: Mapping,
    state: Mapping,
    code: Mapping,
    prologue: Vec<u8>,
    epilogue: Vec<u8>,
    /// The snippet currently in the code page.
    loaded: Option<Vec<u8>>,
}

static SANDBOX: Mutex<Option<Sandbox>> = Mutex::new(None);

/// Runs `f` with exclusive access to the process-wide sandbox.
pub fn with_sandbox<R>(f: impl FnOnce(&mut Sandbox) -> Result<R>) -> Result<R> {
    let mut guard = SANDBOX.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(Sandbox::new()?);
    }
    f(guard.as_mut().unwrap())
}

const GPR64: [AsmRegister64; 16] = [
    rax, rcx, rdx, rbx, rsp, rbp, rsi, rdi, r8, r9, r10, r11, r12, r13, r14, r15,
];

fn slot(off: u64) -> AsmMemoryOperand {
    qword_ptr(SANDBOX_STATE + off)
}

impl Sandbox {
    fn new() -> Result<Sandbox> {
        let data = Mapping::new(Some(SANDBOX_DATA), SANDBOX_DATA_SIZE)?;
        let state = Mapping::new(Some(SANDBOX_STATE), PAGE)?;
        let code = Mapping::new(Some(SANDBOX_CODE), SANDBOX_CODE_SIZE)?;

        // Prologue: save callee-saved registers and the host RSP, load the
        // guest flags and registers, switch to the guest stack.
        let mut a = CodeAssembler::new(64)?;
        for r in [rbx, rbp, r12, r13, r14, r15] {
            a.push(r)?;
        }
        a.mov(slot(HOST_RSP), rsp)?;
        a.push(slot(abi::STATE_RFLAGS))?;
        a.popfq()?;
        for (i, &r) in GPR64.iter().enumerate() {
            if i != 0 && i != 4 {
                a.mov(r, slot(abi::gpr_offset(i as u32)))?;
            }
        }
        a.mov(rsp, slot(abi::gpr_offset(4)))?;
        a.mov(rax, slot(abi::gpr_offset(0)))?;
        let mut prologue = a.assemble(SANDBOX_CODE)?;
        ensure!(prologue.len() <= (SANDBOX_SNIPPET - SANDBOX_CODE) as usize);
        prologue.resize((SANDBOX_SNIPPET - SANDBOX_CODE) as usize, 0x90);

        // Epilogue (position independent): store the guest registers, switch
        // back to the host stack, store the flags, restore and return.
        let mut a = CodeAssembler::new(64)?;
        a.mov(slot(abi::gpr_offset(0)), rax)?;
        a.mov(slot(abi::gpr_offset(4)), rsp)?;
        a.mov(rsp, slot(HOST_RSP))?;
        a.pushfq()?;
        a.pop(slot(abi::STATE_RFLAGS))?;
        for (i, &r) in GPR64.iter().enumerate() {
            if i != 0 && i != 4 {
                a.mov(slot(abi::gpr_offset(i as u32)), r)?;
            }
        }
        for r in [r15, r14, r13, r12, rbp, rbx] {
            a.pop(r)?;
        }
        a.ret()?;
        let epilogue = a.assemble(0)?;
        Ok(Sandbox {
            data,
            state,
            code,
            prologue,
            epilogue,
            loaded: None,
        })
    }

    /// Runs `snippet` (placed at [`SANDBOX_SNIPPET`]) natively from `input`
    /// registers and flags with the data page holding `data` (zero-padded).
    /// The snippet must fall off its end. Returns the registers and flags
    /// afterwards (RIP set to the snippet's end) and the data page.
    pub fn run(&mut self, snippet: &[u8], input: &CpuState, data: &[u8]) -> Result<(CpuState, Vec<u8>)> {
        ensure!(data.len() <= SANDBOX_DATA_SIZE);
        let total = self.prologue.len() + snippet.len() + self.epilogue.len();
        ensure!(total <= SANDBOX_CODE_SIZE, "snippet too large");
        // User mode can't set IF/IOPL; refuse TF/DF/AC and other control bits.
        ensure!(
            input.rflags & !(abi::flags::STATUS | abi::flags::RESERVED1 | abi::flags::IF) == 0,
            "sandbox rflags {:#x} has control bits set",
            input.rflags
        );

        if self.loaded.as_deref() != Some(snippet) {
            self.code.protect(libc::PROT_READ | libc::PROT_WRITE)?;
            let code = self.code.bytes_mut();
            code.fill(0xcc);
            let mut at = 0;
            for part in [&self.prologue[..], snippet, &self.epilogue[..]] {
                code[at..at + part.len()].copy_from_slice(part);
                at += part.len();
            }
            self.code.protect(libc::PROT_READ | libc::PROT_EXEC)?;
            self.loaded = Some(snippet.to_vec());
        }

        let page = self.data.bytes_mut();
        page.fill(0);
        page[..data.len()].copy_from_slice(data);

        let block = input.to_bytes();
        self.state.bytes_mut()[..block.len()].copy_from_slice(&block);

        // SAFETY: the code page holds prologue + snippet + epilogue; the
        // prologue saves the callee-saved registers and the epilogue restores
        // them and returns. The snippet runs on the guest stack in the data
        // page; tests only use snippets that stay inside the sandbox.
        unsafe {
            let f: extern "C" fn() = std::mem::transmute(SANDBOX_CODE as usize);
            f();
        }

        let mut out = CpuState::from_bytes(self.state.bytes());
        out.rip = SANDBOX_SNIPPET + snippet.len() as u64;
        out.exit = abi::EXIT_LEFT_CODE;
        Ok((out, self.data.bytes()[..SANDBOX_DATA_SIZE].to_vec()))
    }
}
