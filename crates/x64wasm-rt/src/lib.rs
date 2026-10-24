//! The run-time translator: x64wasm built for `wasm32-unknown-unknown`
//! (`x64wasm_rt.wasm`), which the JS runtime calls when execution reaches
//! code no module has translated (`docs/design/abi.md`, "Run-time
//! translator"; `docs/design/runtime.md`, "Run loop").
//!
//! Exports (wasm32 only): `memory`, `rt_alloc(size) -> ptr`,
//! `rt_free(ptr, size)` and `rt_translate(request) -> result`. Request
//! handling is target-independent ([`handle`]) and tested natively against a
//! [`VecMemory`]. Bad input never panics: it is an error result (status 1)
//! with a message.

use x64wasm::abi::{
    RT_REQ_CODE_BASE, RT_REQ_CODE_LEN, RT_REQ_CODE_PTR, RT_REQ_ENTRY, RT_REQ_FLAGS,
    RT_REQ_KNOWN_LEN, RT_REQ_KNOWN_PTR, RT_REQUEST_SIZE,
};
use x64wasm::{CodeRegion, Options};

/// Read access to the memory requests point into.
pub trait Memory {
    /// The `len` bytes at `ptr`, or `None` if they aren't all in memory. A
    /// null `ptr` is never valid unless `len` is 0.
    fn slice(&self, ptr: u32, len: usize) -> Option<&[u8]>;
}

/// A byte vector standing in for linear memory: pointer = index.
pub struct VecMemory(pub Vec<u8>);

impl Memory for VecMemory {
    fn slice(&self, ptr: u32, len: usize) -> Option<&[u8]> {
        if len == 0 {
            return Some(&[]);
        }
        if ptr == 0 {
            return None;
        }
        let start = ptr as usize;
        self.0.get(start..start.checked_add(len)?)
    }
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    let mut a = [0u8; 4];
    a.copy_from_slice(&b[at..at + 4]);
    u32::from_le_bytes(a)
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

/// Handles the request at `request_ptr`: returns the v3 module, or the
/// error text.
pub fn handle(mem: &impl Memory, request_ptr: u32) -> Result<Vec<u8>, String> {
    let req = mem
        .slice(request_ptr, RT_REQUEST_SIZE)
        .ok_or_else(|| format!("request at {request_ptr:#x} is outside memory"))?;
    let code_base = u64_at(req, RT_REQ_CODE_BASE);
    let code_ptr = u32_at(req, RT_REQ_CODE_PTR);
    let code_len = u32_at(req, RT_REQ_CODE_LEN);
    let entry = u64_at(req, RT_REQ_ENTRY);
    let known_ptr = u32_at(req, RT_REQ_KNOWN_PTR);
    let known_len = u32_at(req, RT_REQ_KNOWN_LEN);
    let flags = u32_at(req, RT_REQ_FLAGS);
    if flags != 0 {
        return Err(format!("unknown flags {flags:#x}"));
    }
    if code_base.checked_add(code_len.into()).is_none() {
        return Err(format!(
            "code at {code_base:#x}+{code_len:#x} wraps around the address space"
        ));
    }
    let code = mem
        .slice(code_ptr, code_len as usize)
        .ok_or_else(|| format!("code bytes at {code_ptr:#x}+{code_len:#x} are outside memory"))?;
    let known_bytes = (known_len as usize)
        .checked_mul(8)
        .and_then(|n| mem.slice(known_ptr, n))
        .ok_or_else(|| {
            format!("known array at {known_ptr:#x} ({known_len} addresses) is outside memory")
        })?;
    let known: Vec<u64> = known_bytes.chunks_exact(8).map(|c| u64_at(c, 0)).collect();
    if !known.is_sorted() {
        return Err("known addresses are not sorted".into());
    }
    let region = CodeRegion {
        base: code_base,
        bytes: code,
    };
    let t = x64wasm::translate_regions(&[region], &[entry], &known, &Options::default())
        .map_err(|e| e.to_string())?;
    Ok(t.wasm)
}

#[cfg(target_arch = "wasm32")]
mod exports {
    use std::alloc::{Layout, alloc, dealloc};
    use std::sync::Mutex;

    use x64wasm::abi::{
        RT_RES_MSG_LEN, RT_RES_MSG_PTR, RT_RES_STATUS, RT_RES_WASM_LEN, RT_RES_WASM_PTR,
        RT_RESULT_SIZE,
    };

    use super::{Memory, handle};

    const ALIGN: usize = 16;

    /// This module's own linear memory.
    struct Linear;

    impl Memory for Linear {
        fn slice(&self, ptr: u32, len: usize) -> Option<&[u8]> {
            if len == 0 {
                return Some(&[]);
            }
            let size = core::arch::wasm32::memory_size(0).checked_mul(65536)?;
            let end = (ptr as usize).checked_add(len)?;
            if ptr == 0 || end > size {
                return None;
            }
            // SAFETY: the range is inside linear memory and not null; the
            // caller (the JS runtime) wrote it and doesn't touch it during
            // the call.
            Some(unsafe { std::slice::from_raw_parts(ptr as usize as *const u8, len) })
        }
    }

    /// The last result and the buffers it points to, kept until the next call.
    struct Output {
        /// u32 fields, so the result is 4-byte aligned.
        result: [u32; RT_RESULT_SIZE / 4],
        wasm: Vec<u8>,
        msg: String,
    }

    static OUTPUT: Mutex<Option<Box<Output>>> = Mutex::new(None);

    fn layout(size: i32) -> Option<Layout> {
        let size = usize::try_from(size).ok().filter(|&s| s > 0)?;
        Layout::from_size_align(size, ALIGN).ok()
    }

    /// A 16-byte-aligned buffer of `size` bytes; 0 if it can't be allocated.
    #[unsafe(no_mangle)]
    pub extern "C" fn rt_alloc(size: i32) -> i32 {
        match layout(size) {
            // SAFETY: the layout has a nonzero size.
            Some(l) => unsafe { alloc(l) as usize as i32 },
            None => 0,
        }
    }

    /// Frees a buffer from `rt_alloc` (`size` as passed to it).
    #[unsafe(no_mangle)]
    pub extern "C" fn rt_free(ptr: i32, size: i32) {
        if let (Some(l), true) = (layout(size), ptr != 0) {
            // SAFETY: the caller passes a pointer from rt_alloc with its size.
            unsafe { dealloc(ptr as usize as *mut u8, l) }
        }
    }

    /// Translates the request at `request` and returns a pointer to the
    /// result, valid until the next call.
    #[unsafe(no_mangle)]
    pub extern "C" fn rt_translate(request: i32) -> i32 {
        let (wasm, msg, status) = match handle(&Linear, request as u32) {
            Ok(wasm) => (wasm, String::new(), 0u32),
            Err(msg) => (Vec::new(), msg, 1u32),
        };
        let mut out = Box::new(Output {
            result: [0; RT_RESULT_SIZE / 4],
            wasm,
            msg,
        });
        let fields = [
            (RT_RES_STATUS, status),
            (RT_RES_WASM_PTR, out.wasm.as_ptr() as usize as u32),
            (RT_RES_WASM_LEN, out.wasm.len() as u32),
            (RT_RES_MSG_PTR, out.msg.as_ptr() as usize as u32),
            (RT_RES_MSG_LEN, out.msg.len() as u32),
        ];
        for (at, v) in fields {
            out.result[at / 4] = v.to_le();
        }
        let ptr = out.result.as_ptr() as usize as i32;
        let mut slot = OUTPUT.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(out);
        ptr
    }
}
