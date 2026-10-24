//! Request handling of the run-time translator (`abi.md`, "Run-time
//! translator"), natively: requests laid out in a byte vector that stands in
//! for the translator's linear memory.

use x64wasm::abi;
use x64wasm::link::read_blocks;
use x64wasm_rt::{VecMemory, handle};

const CODE_BASE: u64 = 0x1_4000_1000;
/// 0x..1000: call 0x..1006; ret / 0x..1006: nop; nop; ret
const CODE: [u8; 9] = [0xe8, 0x01, 0, 0, 0, 0xc3, 0x90, 0x90, 0xc3];
const CODE_PTR: u32 = 0x100;
const KNOWN_PTR: u32 = 0x200;
const REQ_PTR: u32 = 0x400;

struct Req {
    code_base: u64,
    code_ptr: u32,
    code_len: u32,
    entry: u64,
    known_ptr: u32,
    known_len: u32,
    flags: u32,
}

fn request() -> Req {
    Req {
        code_base: CODE_BASE,
        code_ptr: CODE_PTR,
        code_len: CODE.len() as u32,
        entry: CODE_BASE,
        known_ptr: KNOWN_PTR,
        known_len: 0,
        flags: 0,
    }
}

fn memory(req: &Req, known: &[u64]) -> VecMemory {
    let mut m = vec![0u8; 0x1000];
    m[CODE_PTR as usize..][..CODE.len()].copy_from_slice(&CODE);
    for (i, k) in known.iter().enumerate() {
        m[KNOWN_PTR as usize + 8 * i..][..8].copy_from_slice(&k.to_le_bytes());
    }
    let r = REQ_PTR as usize;
    let mut put = |at: usize, b: &[u8]| m[r + at..][..b.len()].copy_from_slice(b);
    put(abi::RT_REQ_CODE_BASE, &req.code_base.to_le_bytes());
    put(abi::RT_REQ_CODE_PTR, &req.code_ptr.to_le_bytes());
    put(abi::RT_REQ_CODE_LEN, &req.code_len.to_le_bytes());
    put(abi::RT_REQ_ENTRY, &req.entry.to_le_bytes());
    put(abi::RT_REQ_KNOWN_PTR, &req.known_ptr.to_le_bytes());
    put(abi::RT_REQ_KNOWN_LEN, &req.known_len.to_le_bytes());
    put(abi::RT_REQ_FLAGS, &req.flags.to_le_bytes());
    VecMemory(m)
}

fn blocks(wasm: &[u8]) -> Vec<u64> {
    read_blocks(wasm)
        .unwrap()
        .blocks
        .iter()
        .map(|b| b.addr)
        .collect()
}

#[test]
fn translates_from_the_entry() {
    let req = request();
    let wasm = handle(&memory(&req, &[]), REQ_PTR).unwrap();
    assert_eq!(
        blocks(&wasm),
        [CODE_BASE, CODE_BASE + 5, CODE_BASE + 6],
        "the entry, the return site and the call target"
    );
}

#[test]
fn known_addresses_are_not_translated_again() {
    let known = [CODE_BASE + 5, CODE_BASE + 6, CODE_BASE + 0x100];
    let req = Req {
        known_len: known.len() as u32,
        ..request()
    };
    let wasm = handle(&memory(&req, &known), REQ_PTR).unwrap();
    assert_eq!(blocks(&wasm), [CODE_BASE]);
    // From the middle: the fall-through into a known address is a jump to it.
    let req = Req {
        entry: CODE_BASE + 6,
        known_len: 1,
        ..request()
    };
    let wasm = handle(&memory(&req, &[CODE_BASE + 7]), REQ_PTR).unwrap();
    assert_eq!(blocks(&wasm), [CODE_BASE + 6]);
}

fn error(req: &Req, known: &[u64], req_ptr: u32) -> String {
    match handle(&memory(req, known), req_ptr) {
        Ok(_) => panic!("expected an error"),
        Err(msg) => msg,
    }
}

#[test]
fn bad_requests_are_errors() {
    let req = request();
    assert!(error(&req, &[], 0).contains("request"), "null request");
    assert!(
        error(&req, &[], 0xff0).contains("request"),
        "request past the end"
    );
    let e = error(
        &Req {
            entry: CODE_BASE + 0x100,
            ..request()
        },
        &[],
        REQ_PTR,
    );
    assert!(e.contains("outside"), "{e}");
    let e = error(
        &Req {
            code_len: 0x10_0000,
            ..request()
        },
        &[],
        REQ_PTR,
    );
    assert!(e.contains("code"), "{e}");
    let e = error(
        &Req {
            code_ptr: 0,
            ..request()
        },
        &[],
        REQ_PTR,
    );
    assert!(e.contains("code"), "{e}");
    let e = error(
        &Req {
            known_len: 0x1000_0000,
            ..request()
        },
        &[],
        REQ_PTR,
    );
    assert!(e.contains("known"), "{e}");
    let e = error(
        &Req {
            known_len: 2,
            ..request()
        },
        &[CODE_BASE + 6, CODE_BASE + 5],
        REQ_PTR,
    );
    assert!(e.contains("sorted"), "{e}");
    let e = error(
        &Req {
            known_len: 1,
            ..request()
        },
        &[CODE_BASE],
        REQ_PTR,
    );
    assert!(e.contains("already"), "{e}");
    let e = error(
        &Req {
            flags: 1,
            ..request()
        },
        &[],
        REQ_PTR,
    );
    assert!(e.contains("flags"), "{e}");
    let e = error(
        &Req {
            code_base: u64::MAX - 4,
            entry: u64::MAX - 4,
            ..request()
        },
        &[],
        REQ_PTR,
    );
    assert!(e.contains("code"), "{e}");
}

#[test]
fn arbitrary_requests_never_panic() {
    // Every field set to extreme values, one at a time and all together.
    let values = [0u64, 1, 0x7f, 0xfff, 0x1000, u32::MAX as u64, u64::MAX];
    for &v in &values {
        for field in 0..7 {
            let mut req = request();
            match field {
                0 => req.code_base = v,
                1 => req.code_ptr = v as u32,
                2 => req.code_len = v as u32,
                3 => req.entry = v,
                4 => req.known_ptr = v as u32,
                5 => req.known_len = v as u32,
                _ => req.flags = v as u32,
            }
            let _ = handle(&memory(&req, &[]), REQ_PTR);
        }
    }
}
