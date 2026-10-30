//! The L1 fixture format (v2): one JSON file per mnemonic family. The
//! contract is `docs/design/fixtures.md`; in short:
//!
//! ```text
//! {
//!   "header": {...},            CPU model, kernel, date. `record --check` ignores it.
//!   "format": 2,
//!   "family": "add",
//!   "seed": "0x...",
//!   "layout": {...},            CODE_ADDR, DATA_ADDR, ... (must match the oracle constants)
//!   "defaults": {...},          FS/GS bases and FP state of a case that does not list them
//!   "default_gprs": {...},      input value of every register a case does not list
//!   "cases": [
//!     {"form": ..., "asm": ..., "bytes": "48 01 d8", "flags_mask": "0xcd5",
//!      "undefined": {register: mask of undefined result bits},      (optional)
//!      "input":  {"rflags": ..., "gprs": {only registers != default_gprs},
//!                 "fs_base", "gs_base" (optional), "fp": {only fields != defaults},
//!                 "mem": [{"addr", "hex"}, ...]},
//!      "output": {"rip": ..., "rflags": ..., "gprs": {only registers != input},
//!                 "fp": {only fields != input}, "fault": {"kind", "addr"} (optional),
//!                 "mem": [{"addr", "hex"}, ...]}}
//!   ]
//! }
//! ```
//!
//! Memory: the data window is zero except for `input.mem`. Every `input.mem`
//! range is compared after the instruction; `output.mem` lists, in full, each
//! compared range whose contents changed. Nothing outside the compared ranges
//! may change. The instruction bytes sit at `CODE_ADDR`, zeros after them.
//! u64 values are `"0x..."` strings; `hex` byte strings may contain spaces.

use std::fmt::Write as _;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value};
use x64wasm::abi::GPR_NAMES;

use super::{CODE_ADDR, DATA_ADDR, DATA_SIZE, FORMAT, MEMORY_BYTES, STATE_ADDR};

/// A contiguous block of guest memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemBlock {
    pub addr: u64,
    pub bytes: Vec<u8>,
}

impl MemBlock {
    pub fn end(&self) -> u64 {
        self.addr + self.bytes.len() as u64
    }
}

/// Number of [`FpState`] fields: XMM0-15, MXCSR, FCW, FSW, FTW, ST0-7.
pub const FP_FIELDS: usize = 28;
const FP_MXCSR: usize = 16;
const FP_FCW: usize = 17;
const FP_FSW: usize = 18;
const FP_FTW: usize = 19;
const FP_ST: usize = 20;

/// The SSE and x87 state a fixture carries (the FXSAVE subset of
/// `docs/design/fixtures.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FpState {
    pub xmm: [u128; 16],
    pub mxcsr: u32,
    pub fcw: u16,
    /// TOP in bits 11-13.
    pub fsw: u16,
    /// Full tag word: 2 bits per physical register R0-R7 (FSTENV layout).
    pub ftw: u16,
    /// ST(0)-ST(7) in ST order, 80 bits each in the low bits.
    pub st: [u128; 8],
}

impl FpState {
    /// The documented defaults: the state after `fninit`, the power-up MXCSR,
    /// XMM registers 0.
    pub const DEFAULT: FpState = FpState {
        xmm: [0; 16],
        mxcsr: 0x1f80,
        fcw: 0x037f,
        fsw: 0,
        ftw: 0xffff,
        st: [0; 8],
    };
    /// All fields 0 (the empty mask).
    pub const ZERO: FpState = FpState {
        xmm: [0; 16],
        mxcsr: 0,
        fcw: 0,
        fsw: 0,
        ftw: 0,
        st: [0; 8],
    };

    /// The field's name in fixture files (`xmm3`, `mxcsr`, `st0`, ...).
    pub fn field_name(i: usize) -> String {
        match i {
            0..16 => format!("xmm{i}"),
            FP_MXCSR => "mxcsr".into(),
            FP_FCW => "fcw".into(),
            FP_FSW => "fsw".into(),
            FP_FTW => "ftw".into(),
            _ => format!("st{}", i - FP_ST),
        }
    }

    /// Width of field `i` in bits.
    pub fn field_bits(i: usize) -> u32 {
        match i {
            0..16 => 128,
            FP_MXCSR => 32,
            FP_FCW | FP_FSW | FP_FTW => 16,
            _ => 80,
        }
    }

    pub fn get(&self, i: usize) -> u128 {
        match i {
            0..16 => self.xmm[i],
            FP_MXCSR => self.mxcsr.into(),
            FP_FCW => self.fcw.into(),
            FP_FSW => self.fsw.into(),
            FP_FTW => self.ftw.into(),
            _ => self.st[i - FP_ST],
        }
    }

    /// Sets field `i` to `v`, truncated to the field's width.
    pub fn set(&mut self, i: usize, v: u128) {
        let v = v & width_mask(Self::field_bits(i));
        match i {
            0..16 => self.xmm[i] = v,
            FP_MXCSR => self.mxcsr = v as u32,
            FP_FCW => self.fcw = v as u16,
            FP_FSW => self.fsw = v as u16,
            FP_FTW => self.ftw = v as u16,
            _ => self.st[i - FP_ST] = v,
        }
    }

    /// x87 TOP (FSW bits 11-13).
    pub fn top(&self) -> usize {
        usize::from((self.fsw >> 11) & 7)
    }

    /// The physical registers R0-R7 (state block order): R[(TOP + i) mod 8] = ST(i).
    pub fn physical(&self) -> [u128; 8] {
        let top = self.top();
        std::array::from_fn(|p| self.st[(p + 8 - top) % 8])
    }

    /// Sets ST(0)-ST(7) from the physical registers, with the current TOP.
    pub fn set_physical(&mut self, r: &[u128; 8]) {
        let top = self.top();
        self.st = std::array::from_fn(|i| r[(top + i) % 8]);
    }
}

impl Default for FpState {
    fn default() -> Self {
        FpState::DEFAULT
    }
}

fn width_mask(bits: u32) -> u128 {
    if bits == 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    }
}

/// x87 tag of a non-empty register from its 80-bit contents, as FSTENV
/// reports it: 0 valid, 1 zero, 2 special (SDM Vol. 1, FXSAVE tag word).
pub fn x87_tag(value: u128) -> u16 {
    let exponent = (value >> 64) & 0x7fff;
    let significand = value as u64;
    match exponent {
        0x7fff => 2,
        0 if significand == 0 => 1,
        0 => 2,
        _ if significand >> 63 == 1 => 0,
        _ => 2,
    }
}

/// FXSAVE's abridged tag (bit p set: R[p] not empty) of a full tag word.
pub fn abridged_tag(ftw: u16) -> u8 {
    (0..8)
        .filter(|p| (ftw >> (2 * p)) & 3 != 3)
        .fold(0, |acc, p| acc | 1 << p)
}

/// The full tag word FSTENV reports for an abridged tag and the registers
/// of `fp` (its ST values and TOP).
pub fn full_tag_word(abridged: u8, fp: &FpState) -> u16 {
    let r = fp.physical();
    (0..8).fold(0, |acc, p| {
        let tag = if abridged >> p & 1 == 0 {
            3
        } else {
            x87_tag(r[p])
        };
        acc | tag << (2 * p)
    })
}

/// An exception raised by the instruction (the expected outcome).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultKind {
    /// #DE (`SIGFPE`, `FPE_INTDIV`).
    De,
    /// #UD (`SIGILL`, `ILL_ILLOPN`).
    Ud,
    /// #GP (`SIGSEGV`, `SI_KERNEL`).
    Gp,
    /// #PF (`SIGSEGV`, `SEGV_MAPERR` or `SEGV_ACCERR`), with the address.
    Pf,
}

impl FaultKind {
    pub const ALL: [FaultKind; 4] = [FaultKind::De, FaultKind::Ud, FaultKind::Gp, FaultKind::Pf];

    pub fn name(self) -> &'static str {
        match self {
            FaultKind::De => "DE",
            FaultKind::Ud => "UD",
            FaultKind::Gp => "GP",
            FaultKind::Pf => "PF",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub kind: FaultKind,
    /// The faulting address for `Pf`, else 0.
    pub addr: u64,
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            FaultKind::Pf => write!(f, "#PF at {:#x}", self.addr),
            k => write!(f, "#{}", k.name()),
        }
    }
}

/// Result bits the SDM leaves undefined, per register and memory byte (not
/// compared; the file holds their input values). All zero and no memory
/// masks: everything is defined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Undefined {
    pub gprs: [u64; 16],
    pub fp: FpState,
    /// Bit masks over memory bytes, each inside one compared range.
    pub mem: Vec<MemBlock>,
}

impl Undefined {
    pub const NONE: Undefined = Undefined {
        gprs: [0; 16],
        fp: FpState::ZERO,
        mem: Vec::new(),
    };

    /// The memory mask over the bytes of `block` (0 where nothing is masked).
    pub fn mem_mask(&self, block: &MemBlock) -> Vec<u8> {
        let mut out = vec![0u8; block.bytes.len()];
        for m in &self.mem {
            for (k, &bits) in m.bytes.iter().enumerate() {
                let a = m.addr + k as u64;
                if a >= block.addr && a < block.end() {
                    out[(a - block.addr) as usize] |= bits;
                }
            }
        }
        out
    }

    /// Checks that every memory mask lies inside one of `ranges`.
    pub fn check_mem(&self, ranges: &[MemBlock]) -> Result<()> {
        for m in &self.mem {
            ensure!(
                ranges
                    .iter()
                    .any(|r| m.addr >= r.addr && m.end() <= r.end()),
                "undefined.mem at {:#x} (+{}) is not inside a compared range",
                m.addr,
                m.bytes.len()
            );
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::NONE
    }
}

impl Default for Undefined {
    fn default() -> Self {
        Undefined::NONE
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    /// All 16 registers (the file lists only those that differ from `default_gprs`).
    pub gprs: [u64; 16],
    pub rflags: u64,
    pub fs_base: u64,
    pub gs_base: u64,
    /// The file lists only the fields that differ from [`FpState::DEFAULT`].
    pub fp: FpState,
    /// Initial contents of the compared ranges, sorted and disjoint.
    pub mem: Vec<MemBlock>,
}

impl Input {
    /// Registers, RFLAGS and memory as given; FS/GS bases and FP state at
    /// their defaults.
    pub fn new(gprs: [u64; 16], rflags: u64, mem: Vec<MemBlock>) -> Self {
        Input {
            gprs,
            rflags,
            fs_base: 0,
            gs_base: 0,
            fp: FpState::DEFAULT,
            mem,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    /// All 16 registers (the file lists only those that differ from the input).
    pub gprs: [u64; 16],
    pub rip: u64,
    pub rflags: u64,
    /// The file lists only the fields that differ from the input.
    pub fp: FpState,
    pub fault: Option<Fault>,
    /// Final contents of every compared range, in `Input::mem` order (the
    /// file lists only the ranges that changed).
    pub mem: Vec<MemBlock>,
}

impl Output {
    /// A placeholder output (generated inputs before recording).
    pub fn empty() -> Self {
        Output {
            gprs: [0; 16],
            rip: 0,
            rflags: 0,
            fp: FpState::ZERO,
            fault: None,
            mem: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    pub form: String,
    pub bytes: Vec<u8>,
    pub asm: String,
    /// RFLAGS bits whose output value is architecturally defined and compared.
    pub flags_mask: u64,
    /// Register bits whose output value is undefined and not compared.
    pub undefined: Undefined,
    pub input: Input,
    pub output: Output,
}

impl Case {
    /// Replaces every output bit outside `flags_mask` and inside `undefined`
    /// with its input value, so that the file does not depend on what a
    /// particular CPU model leaves in undefined results.
    pub fn normalize(&mut self) {
        let (i, o, u) = (&self.input, &mut self.output, &self.undefined);
        o.rflags = (o.rflags & self.flags_mask) | (i.rflags & !self.flags_mask);
        for r in 0..16 {
            o.gprs[r] = (o.gprs[r] & !u.gprs[r]) | (i.gprs[r] & u.gprs[r]);
        }
        for f in 0..FP_FIELDS {
            let m = u.fp.get(f);
            o.fp.set(f, (o.fp.get(f) & !m) | (i.fp.get(f) & m));
        }
        for (ob, ib) in o.mem.iter_mut().zip(&i.mem) {
            let mask = u.mem_mask(ib);
            for (k, &m) in mask.iter().enumerate() {
                ob.bytes[k] = (ob.bytes[k] & !m) | (ib.bytes[k] & m);
            }
        }
    }
}

/// Recording metadata. Ignored by `record --check`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub cpu: String,
    pub kernel: String,
    pub recorded: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fixture {
    pub family: String,
    pub seed: u64,
    pub default_gprs: [u64; 16],
    pub cases: Vec<Case>,
}

pub const HEADER_NOTE: &str = "Recorded natively by `testkit record`; never edit by hand. `record --check` ignores this header.";

fn hex(v: u64) -> String {
    format!("{v:#x}")
}

/// An FP field value: XMM and ST padded to their full width.
fn fp_hex(field: usize, v: u128) -> String {
    match FpState::field_bits(field) {
        128 => format!("0x{v:032x}"),
        80 => format!("0x{v:020x}"),
        _ => format!("{v:#x}"),
    }
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).expect("string serializes")
}

/// Instruction bytes: `"48 01 d8"`.
fn bytes_hex(bytes: &[u8]) -> String {
    let parts: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    parts.join(" ")
}

/// Memory bytes: groups of 8 bytes separated by spaces.
fn mem_hex(bytes: &[u8]) -> String {
    let parts: Vec<String> = bytes
        .chunks(8)
        .map(|chunk| chunk.iter().map(|b| format!("{b:02x}")).collect())
        .collect();
    parts.join(" ")
}

fn gpr_object(entries: &[(usize, u64)]) -> String {
    let parts: Vec<String> = entries
        .iter()
        .map(|&(i, v)| format!("{}: {}", json_str(GPR_NAMES[i]), json_str(&hex(v))))
        .collect();
    format!("{{{}}}", parts.join(", "))
}

/// The `fp` fields of `fp` that differ from `base`, or `None` if none do.
fn fp_object(fp: &FpState, base: &FpState) -> Option<String> {
    let parts: Vec<String> = (0..FP_FIELDS)
        .filter(|&f| fp.get(f) != base.get(f))
        .map(|f| {
            format!(
                "{}: {}",
                json_str(&FpState::field_name(f)),
                json_str(&fp_hex(f, fp.get(f)))
            )
        })
        .collect();
    (!parts.is_empty()).then(|| format!("{{{}}}", parts.join(", ")))
}

fn undefined_object(u: &Undefined) -> Option<String> {
    let mut parts: Vec<String> = (0..16)
        .filter(|&i| u.gprs[i] != 0)
        .map(|i| format!("{}: {}", json_str(GPR_NAMES[i]), json_str(&hex(u.gprs[i]))))
        .collect();
    parts.extend((0..FP_FIELDS).filter(|&f| u.fp.get(f) != 0).map(|f| {
        format!(
            "{}: {}",
            json_str(&FpState::field_name(f)),
            json_str(&format!("{:#x}", u.fp.get(f)))
        )
    }));
    if !u.mem.is_empty() {
        let blocks: Vec<String> = u
            .mem
            .iter()
            .map(|b| {
                format!(
                    "{{\"addr\": {}, \"hex\": {}}}",
                    json_str(&hex(b.addr)),
                    json_str(&mem_hex(&b.bytes))
                )
            })
            .collect();
        parts.push(format!("\"mem\": [{}]", blocks.join(", ")));
    }
    (!parts.is_empty()).then(|| format!("{{{}}}", parts.join(", ")))
}

fn fault_object(f: &Fault) -> String {
    match f.kind {
        FaultKind::Pf => format!(
            "{{\"kind\": {}, \"addr\": {}}}",
            json_str(f.kind.name()),
            json_str(&hex(f.addr))
        ),
        k => format!("{{\"kind\": {}}}", json_str(k.name())),
    }
}

fn mem_list(blocks: &[&MemBlock], indent: &str) -> String {
    if blocks.is_empty() {
        return "[]".to_string();
    }
    let parts: Vec<String> = blocks
        .iter()
        .map(|b| {
            format!(
                "{{\"addr\": {}, \"hex\": {}}}",
                json_str(&hex(b.addr)),
                json_str(&mem_hex(&b.bytes))
            )
        })
        .collect();
    format!("[{}]", parts.join(&format!(",\n{indent}")))
}

/// The `defaults` object: values of fields a case does not list.
fn defaults_json() -> String {
    let d = FpState::DEFAULT;
    format!(
        "{{\"fs_base\": \"0x0\", \"gs_base\": \"0x0\", \"xmm\": \"{:#x}\", \"mxcsr\": \"{:#x}\", \"fcw\": \"{:#x}\", \"fsw\": \"{:#x}\", \"ftw\": \"{:#x}\", \"st\": \"{:#x}\"}}",
        d.xmm[0], d.mxcsr, d.fcw, d.fsw, d.ftw, d.st[0]
    )
}

/// Serializes a fixture with a stable, review-friendly layout.
pub fn to_json(fixture: &Fixture, header: &Header) -> String {
    let mut out = String::new();
    out.push_str("{\n  \"header\": {\n");
    let _ = writeln!(out, "    \"cpu\": {},", json_str(&header.cpu));
    let _ = writeln!(out, "    \"kernel\": {},", json_str(&header.kernel));
    let _ = writeln!(out, "    \"recorded\": {},", json_str(&header.recorded));
    let _ = writeln!(out, "    \"note\": {}", json_str(HEADER_NOTE));
    out.push_str("  },\n");
    let _ = writeln!(out, "  \"format\": {FORMAT},");
    let _ = writeln!(out, "  \"family\": {},", json_str(&fixture.family));
    let _ = writeln!(out, "  \"seed\": {},", json_str(&hex(fixture.seed)));
    let _ = writeln!(
        out,
        "  \"layout\": {{\"code\": \"{CODE_ADDR:#x}\", \"data\": \"{DATA_ADDR:#x}\", \"data_len\": \"{DATA_SIZE:#x}\", \"state\": \"{STATE_ADDR:#x}\", \"memory\": \"{MEMORY_BYTES:#x}\"}},"
    );
    let _ = writeln!(out, "  \"defaults\": {},", defaults_json());
    out.push_str("  \"default_gprs\": {\n");
    for row in 0..4 {
        let parts: Vec<String> = (row * 4..row * 4 + 4)
            .map(|i| {
                format!(
                    "{}: {}",
                    json_str(GPR_NAMES[i]),
                    json_str(&hex(fixture.default_gprs[i]))
                )
            })
            .collect();
        let sep = if row == 3 { "" } else { "," };
        let _ = writeln!(out, "    {}{sep}", parts.join(", "));
    }
    out.push_str("  },\n  \"cases\": [\n");
    for (n, case) in fixture.cases.iter().enumerate() {
        let in_gprs: Vec<(usize, u64)> = (0..16)
            .filter(|&i| case.input.gprs[i] != fixture.default_gprs[i])
            .map(|i| (i, case.input.gprs[i]))
            .collect();
        let out_gprs: Vec<(usize, u64)> = (0..16)
            .filter(|&i| case.output.gprs[i] != case.input.gprs[i])
            .map(|i| (i, case.output.gprs[i]))
            .collect();
        let in_mem: Vec<&MemBlock> = case.input.mem.iter().collect();
        let out_mem: Vec<&MemBlock> = case
            .output
            .mem
            .iter()
            .zip(&case.input.mem)
            .filter(|(o, i)| o.bytes != i.bytes)
            .map(|(o, _)| o)
            .collect();
        let _ = write!(
            out,
            "    {{\"form\": {}, \"asm\": {}, \"bytes\": {}, \"flags_mask\": {},",
            json_str(&case.form),
            json_str(&case.asm),
            json_str(&bytes_hex(&case.bytes)),
            json_str(&hex(case.flags_mask))
        );
        if let Some(u) = undefined_object(&case.undefined) {
            let _ = write!(out, "\n     \"undefined\": {u},");
        }
        out.push('\n');
        let _ = write!(
            out,
            "     \"input\": {{\"rflags\": {}, \"gprs\": {},",
            json_str(&hex(case.input.rflags)),
            gpr_object(&in_gprs)
        );
        for (key, v) in [
            ("fs_base", case.input.fs_base),
            ("gs_base", case.input.gs_base),
        ] {
            if v != 0 {
                let _ = write!(out, " {}: {},", json_str(key), json_str(&hex(v)));
            }
        }
        if let Some(fp) = fp_object(&case.input.fp, &FpState::DEFAULT) {
            let _ = write!(out, "\n               \"fp\": {fp},");
        }
        if in_mem.is_empty() {
            out.push_str(" \"mem\": []},\n");
        } else {
            let indent = "                ";
            let _ = writeln!(
                out,
                "\n               \"mem\": {}}},",
                mem_list(&in_mem, indent)
            );
        }
        let _ = write!(
            out,
            "     \"output\": {{\"rip\": {}, \"rflags\": {}, \"gprs\": {},",
            json_str(&hex(case.output.rip)),
            json_str(&hex(case.output.rflags)),
            gpr_object(&out_gprs)
        );
        if let Some(fault) = &case.output.fault {
            let _ = write!(out, " \"fault\": {},", fault_object(fault));
        }
        if let Some(fp) = fp_object(&case.output.fp, &case.input.fp) {
            let _ = write!(out, "\n                \"fp\": {fp},");
        }
        let sep = if n + 1 == fixture.cases.len() {
            ""
        } else {
            ","
        };
        if out_mem.is_empty() {
            let _ = writeln!(out, " \"mem\": []}}}}{sep}");
        } else {
            let indent = "                 ";
            let _ = writeln!(
                out,
                "\n                \"mem\": {}}}}}{sep}",
                mem_list(&out_mem, indent)
            );
        }
    }
    out.push_str("  ]\n}\n");
    out
}

fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    v.get(key).with_context(|| format!("missing field `{key}`"))
}

fn str_field<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    field(v, key)?
        .as_str()
        .with_context(|| format!("field `{key}` is not a string"))
}

fn parse_u128(text: &str) -> Result<u128> {
    let digits = text
        .strip_prefix("0x")
        .with_context(|| format!("`{text}` is not a 0x-prefixed hex string"))?;
    u128::from_str_radix(digits, 16).with_context(|| format!("bad hex value `{text}`"))
}

fn parse_u64(text: &str) -> Result<u64> {
    u64::try_from(parse_u128(text)?).with_context(|| format!("`{text}` exceeds 64 bits"))
}

fn u64_field(v: &Value, key: &str) -> Result<u64> {
    parse_u64(str_field(v, key)?).with_context(|| format!("field `{key}`"))
}

/// An optional u64 field, `default` when absent.
fn opt_u64_field(v: &Value, key: &str, default: u64) -> Result<u64> {
    match v.get(key) {
        None => Ok(default),
        Some(_) => u64_field(v, key),
    }
}

fn parse_bytes(text: &str) -> Result<Vec<u8>> {
    let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    ensure!(
        digits.len().is_multiple_of(2),
        "odd number of hex digits in `{text}`"
    );
    digits
        .chunks(2)
        .map(|pair| {
            let s = std::str::from_utf8(pair)?;
            u8::from_str_radix(s, 16).with_context(|| format!("bad hex byte `{s}`"))
        })
        .collect()
}

fn parse_gprs(v: &Value, base: &[u64; 16]) -> Result<[u64; 16]> {
    let mut gprs = *base;
    let obj = v.as_object().context("`gprs` is not an object")?;
    for (name, value) in obj {
        let index = GPR_NAMES
            .iter()
            .position(|n| n == name)
            .with_context(|| format!("unknown register `{name}`"))?;
        let text = value.as_str().context("register value is not a string")?;
        gprs[index] = parse_u64(text)?;
    }
    Ok(gprs)
}

fn fp_index(name: &str) -> Option<usize> {
    (0..FP_FIELDS).find(|&f| FpState::field_name(f) == name)
}

/// Parses a field value that must fit the field's width.
fn fp_value(f: usize, value: &Value) -> Result<u128> {
    let text = value.as_str().context("value is not a string")?;
    let v = parse_u128(text)?;
    ensure!(
        v & !width_mask(FpState::field_bits(f)) == 0,
        "`{text}` exceeds the {} bits of `{}`",
        FpState::field_bits(f),
        FpState::field_name(f)
    );
    Ok(v)
}

/// An optional `fp` object applied on top of `base`.
fn parse_fp(v: Option<&Value>, base: &FpState) -> Result<FpState> {
    let mut fp = *base;
    let Some(v) = v else { return Ok(fp) };
    for (name, value) in v.as_object().context("`fp` is not an object")? {
        let f = fp_index(name).with_context(|| format!("unknown fp field `{name}`"))?;
        fp.set(f, fp_value(f, value).with_context(|| format!("fp.{name}"))?);
    }
    Ok(fp)
}

fn parse_undefined(v: Option<&Value>) -> Result<Undefined> {
    let mut u = Undefined::NONE;
    let Some(v) = v else { return Ok(u) };
    let obj: &Map<String, Value> = v.as_object().context("`undefined` is not an object")?;
    for (name, value) in obj {
        if name == "mem" {
            u.mem = parse_mem(value).context("undefined.mem")?;
            ensure!(
                u.mem.iter().all(|m| !m.bytes.is_empty()),
                "undefined.mem: empty mask"
            );
        } else if let Some(i) = GPR_NAMES.iter().position(|n| n == name) {
            u.gprs[i] = parse_u64(value.as_str().context("mask is not a string")?)?;
        } else {
            let f = fp_index(name).with_context(|| format!("unknown register `{name}`"))?;
            u.fp.set(f, fp_value(f, value)?);
        }
    }
    Ok(u)
}

fn parse_fault(v: Option<&Value>) -> Result<Option<Fault>> {
    let Some(v) = v else { return Ok(None) };
    let name = str_field(v, "kind")?;
    let kind = FaultKind::ALL
        .into_iter()
        .find(|k| k.name() == name)
        .with_context(|| format!("unknown fault kind `{name}`"))?;
    let addr = if kind == FaultKind::Pf {
        u64_field(v, "addr")?
    } else {
        ensure!(v.get("addr").is_none(), "only PF faults carry an address");
        0
    };
    Ok(Some(Fault { kind, addr }))
}

fn parse_mem(v: &Value) -> Result<Vec<MemBlock>> {
    v.as_array()
        .context("`mem` is not an array")?
        .iter()
        .map(|b| {
            Ok(MemBlock {
                addr: u64_field(b, "addr")?,
                bytes: parse_bytes(str_field(b, "hex")?)?,
            })
        })
        .collect()
}

/// Parses a fixture file. Fails if its format, layout or defaults differ
/// from this build's.
pub fn parse(text: &str) -> Result<(Header, Fixture)> {
    let root: Value = serde_json::from_str(text).context("fixture is not valid JSON")?;
    let format = field(&root, "format")?
        .as_u64()
        .context("`format` is not a number")?;
    ensure!(
        format == u64::from(FORMAT),
        "fixture format {format}, this build reads {FORMAT}; re-record"
    );
    let header_v = field(&root, "header")?;
    let header = Header {
        cpu: str_field(header_v, "cpu").unwrap_or_default().to_string(),
        kernel: str_field(header_v, "kernel")
            .unwrap_or_default()
            .to_string(),
        recorded: str_field(header_v, "recorded")
            .unwrap_or_default()
            .to_string(),
    };
    let layout = field(&root, "layout")?;
    for (key, want) in [
        ("code", CODE_ADDR),
        ("data", DATA_ADDR),
        ("data_len", DATA_SIZE),
        ("state", STATE_ADDR),
        ("memory", MEMORY_BYTES),
    ] {
        let got = u64_field(layout, key)?;
        ensure!(
            got == want,
            "layout.{key} is {got:#x} but the oracle uses {want:#x}; re-record"
        );
    }
    let defaults: Value = serde_json::from_str(&defaults_json()).expect("defaults are JSON");
    ensure!(
        field(&root, "defaults")? == &defaults,
        "`defaults` differ from this build's {defaults}; re-record"
    );
    let default_gprs = parse_gprs(field(&root, "default_gprs")?, &[0; 16])?;
    let mut cases = Vec::new();
    for (n, c) in field(&root, "cases")?
        .as_array()
        .context("`cases` is not an array")?
        .iter()
        .enumerate()
    {
        let parsed = (|| -> Result<Case> {
            let input_v = field(c, "input")?;
            let output_v = field(c, "output")?;
            let input = Input {
                gprs: parse_gprs(field(input_v, "gprs")?, &default_gprs)?,
                rflags: u64_field(input_v, "rflags")?,
                fs_base: opt_u64_field(input_v, "fs_base", 0)?,
                gs_base: opt_u64_field(input_v, "gs_base", 0)?,
                fp: parse_fp(input_v.get("fp"), &FpState::DEFAULT)?,
                mem: parse_mem(field(input_v, "mem")?)?,
            };
            let mut out_mem = input.mem.clone();
            for block in parse_mem(field(output_v, "mem")?)? {
                let Some(slot) = out_mem
                    .iter_mut()
                    .find(|b| b.addr == block.addr && b.bytes.len() == block.bytes.len())
                else {
                    bail!(
                        "output.mem block at {:#x} (+{}) is not one of the input ranges",
                        block.addr,
                        block.bytes.len()
                    );
                };
                *slot = block;
            }
            let undefined = parse_undefined(c.get("undefined"))?;
            undefined.check_mem(&input.mem)?;
            Ok(Case {
                form: str_field(c, "form")?.to_string(),
                bytes: parse_bytes(str_field(c, "bytes")?)?,
                asm: str_field(c, "asm")?.to_string(),
                flags_mask: u64_field(c, "flags_mask")?,
                undefined,
                output: Output {
                    gprs: parse_gprs(field(output_v, "gprs")?, &input.gprs)?,
                    rip: u64_field(output_v, "rip")?,
                    rflags: u64_field(output_v, "rflags")?,
                    fp: parse_fp(output_v.get("fp"), &input.fp)?,
                    fault: parse_fault(output_v.get("fault"))?,
                    mem: out_mem,
                },
                input,
            })
        })()
        .with_context(|| format!("case {n}"))?;
        cases.push(parsed);
    }
    Ok((
        header,
        Fixture {
            family: str_field(&root, "family")?.to_string(),
            seed: u64_field(&root, "seed")?,
            default_gprs,
            cases,
        },
    ))
}

fn fmt_gprs(gprs: &[u64; 16]) -> String {
    let parts: Vec<String> = (0..16)
        .map(|i| format!("{}={:#x}", GPR_NAMES[i], gprs[i]))
        .collect();
    parts.join(" ")
}

fn fmt_fp(fp: &FpState) -> String {
    let parts: Vec<String> = (0..FP_FIELDS)
        .map(|f| format!("{}={}", FpState::field_name(f), fp_hex(f, fp.get(f))))
        .collect();
    parts.join(" ")
}

fn fmt_mem(mem: &[MemBlock]) -> String {
    let parts: Vec<String> = mem
        .iter()
        .map(|b| format!("{:#x}:[{}]", b.addr, mem_hex(&b.bytes)))
        .collect();
    parts.join(" ")
}

fn fmt_fault(f: &Option<Fault>) -> String {
    f.map_or_else(|| "none".to_string(), |f| f.to_string())
}

/// Lists every non-header difference between a committed fixture and a fresh
/// recording, as readable lines.
pub fn diff(committed: &Fixture, recorded: &Fixture) -> Vec<String> {
    let mut out = Vec::new();
    let family = &recorded.family;
    if committed.family != recorded.family {
        out.push(format!(
            "{family}: family is `{}`, recorded `{}`",
            committed.family, recorded.family
        ));
    }
    if committed.seed != recorded.seed {
        out.push(format!(
            "{family}: seed is {:#x}, recorded {:#x}",
            committed.seed, recorded.seed
        ));
    }
    if committed.default_gprs != recorded.default_gprs {
        out.push(format!(
            "{family}: default_gprs differ\n    committed: {}\n    recorded:  {}",
            fmt_gprs(&committed.default_gprs),
            fmt_gprs(&recorded.default_gprs)
        ));
    }
    if committed.cases.len() != recorded.cases.len() {
        out.push(format!(
            "{family}: {} cases committed, {} recorded",
            committed.cases.len(),
            recorded.cases.len()
        ));
    }
    for (n, (a, b)) in committed.cases.iter().zip(&recorded.cases).enumerate() {
        let at = format!("{family}[{n}] {} `{}`", b.form, b.asm);
        let mut field = |name: &str, x: String, y: String| {
            if x != y {
                out.push(format!(
                    "{at}: {name}\n    committed: {x}\n    recorded:  {y}"
                ));
            }
        };
        field("form", a.form.clone(), b.form.clone());
        field("bytes", bytes_hex(&a.bytes), bytes_hex(&b.bytes));
        field("asm", a.asm.clone(), b.asm.clone());
        field("flags_mask", hex(a.flags_mask), hex(b.flags_mask));
        field(
            "undefined",
            format!("{:?}", undefined_object(&a.undefined)),
            format!("{:?}", undefined_object(&b.undefined)),
        );
        field(
            "input.gprs",
            fmt_gprs(&a.input.gprs),
            fmt_gprs(&b.input.gprs),
        );
        field("input.rflags", hex(a.input.rflags), hex(b.input.rflags));
        field("input.fs_base", hex(a.input.fs_base), hex(b.input.fs_base));
        field("input.gs_base", hex(a.input.gs_base), hex(b.input.gs_base));
        field("input.fp", fmt_fp(&a.input.fp), fmt_fp(&b.input.fp));
        field("input.mem", fmt_mem(&a.input.mem), fmt_mem(&b.input.mem));
        field(
            "output.gprs",
            fmt_gprs(&a.output.gprs),
            fmt_gprs(&b.output.gprs),
        );
        field("output.rip", hex(a.output.rip), hex(b.output.rip));
        field("output.rflags", hex(a.output.rflags), hex(b.output.rflags));
        field("output.fp", fmt_fp(&a.output.fp), fmt_fp(&b.output.fp));
        field(
            "output.fault",
            fmt_fault(&a.output.fault),
            fmt_fault(&b.output.fault),
        );
        field("output.mem", fmt_mem(&a.output.mem), fmt_mem(&b.output.mem));
    }
    out
}

/// Like [`diff`], but only for the fields the generator decides (everything
/// except `output`). Used to check fixtures on machines that cannot record.
pub fn diff_inputs(committed: &Fixture, generated: &Fixture) -> Vec<String> {
    let strip = |f: &Fixture| {
        let mut f = f.clone();
        for case in &mut f.cases {
            case.output = Output::empty();
        }
        f
    };
    diff(&strip(committed), &strip(generated))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Fixture {
        let mut gprs = [0u64; 16];
        gprs[3] = 0x1234;
        let mut out_gprs = gprs;
        out_gprs[0] = u64::MAX;
        let input_mem = vec![
            MemBlock {
                addr: DATA_ADDR + 0x10,
                bytes: (0..20).collect(),
            },
            MemBlock {
                addr: DATA_ADDR + 0x100,
                bytes: vec![1, 2, 3],
            },
        ];
        let mut output_mem = input_mem.clone();
        output_mem[1].bytes[0] = 9;
        let plain = Case {
            form: "add_m8_r8".into(),
            bytes: vec![0x00, 0x18],
            asm: "add [rax], bl".into(),
            flags_mask: 0xcd5,
            undefined: Undefined::NONE,
            input: Input::new(gprs, 0x202, input_mem),
            output: Output {
                gprs: out_gprs,
                rip: CODE_ADDR + 2,
                rflags: 0x246,
                fp: FpState::DEFAULT,
                fault: None,
                mem: output_mem,
            },
        };
        // Every v2 field set to something other than its default.
        let mut fp_in = FpState::DEFAULT;
        fp_in.xmm[1] = 0x0011_2233_4455_6677_8899_aabb_ccdd_eeff;
        fp_in.xmm[15] = 1;
        fp_in.mxcsr = 0x9fc0;
        fp_in.fcw = 0x027f;
        fp_in.fsw = 0x3800;
        fp_in.ftw = 0x3fff;
        fp_in.st[0] = 0x3fff_8000_0000_0000_0000;
        let mut fp_out = fp_in;
        fp_out.xmm[2] = u128::MAX;
        fp_out.fsw = 0;
        fp_out.ftw = 0xffff;
        let mut undefined = Undefined::NONE;
        undefined.gprs[2] = 0xffff_ffff;
        undefined.fp.xmm[3] = 0xff << 120;
        undefined.fp.fsw = 0x4700;
        undefined.mem = vec![MemBlock {
            addr: DATA_ADDR + 0x14,
            bytes: vec![0xff, 0x0f],
        }];
        let mut rich = plain.clone();
        rich.form = "pxor_xmm_xmmm128/reg".into();
        rich.undefined = undefined;
        rich.input.fs_base = DATA_ADDR;
        rich.input.gs_base = 0x7ffe_0000;
        rich.input.fp = fp_in;
        rich.output.fp = fp_out;
        rich.output.rip = CODE_ADDR;
        rich.output.fault = Some(Fault {
            kind: FaultKind::Pf,
            addr: 0x1000,
        });
        let mut de = plain.clone();
        de.output.fault = Some(Fault {
            kind: FaultKind::De,
            addr: 0,
        });
        Fixture {
            family: "add".into(),
            seed: 42,
            default_gprs: [7; 16],
            cases: vec![plain, rich, de],
        }
    }

    #[test]
    fn round_trips() {
        let fixture = sample();
        let header = Header {
            cpu: "test \"cpu\"".into(),
            kernel: "k".into(),
            recorded: "2026-01-01".into(),
        };
        let text = to_json(&fixture, &header);
        let (h, f) = parse(&text).unwrap();
        assert_eq!(h, header);
        assert_eq!(f, fixture);
        assert!(diff(&f, &fixture).is_empty());
        // Defaults are not written; v2 fields only where set.
        assert_eq!(text.matches("\"mxcsr\": \"0x1f80\"").count(), 1, "{text}");
        assert!(text.contains("\"xmm1\": \"0x00112233445566778899aabbccddeeff\""));
        assert!(text.contains("\"st0\": \"0x3fff8000000000000000\""));
        assert!(text.contains("\"fault\": {\"kind\": \"PF\", \"addr\": \"0x1000\"}"));
        assert!(text.contains("\"fault\": {\"kind\": \"DE\"}"));
        assert!(text.contains("\"undefined\": {\"rdx\": \"0xffffffff\""));
        assert!(text.contains("\"fs_base\": \"0x200000\""));
        assert!(
            text.contains("\"mem\": [{\"addr\": \"0x200014\", \"hex\": \"ff0f\"}]"),
            "{text}"
        );

        let mut changed = fixture.clone();
        changed.cases[0].output.rflags ^= 1;
        let d = diff(&fixture, &changed);
        assert_eq!(d.len(), 1);
        assert!(d[0].contains("output.rflags"), "{d:?}");
        assert!(diff_inputs(&fixture, &changed).is_empty());
        changed.cases[0].input.mem[0].bytes[3] ^= 0xff;
        assert_eq!(diff_inputs(&fixture, &changed).len(), 1);

        for (mutate, name) in [
            (
                (|c: &mut Case| c.output.fp.xmm[4] = 5) as fn(&mut Case),
                "output.fp",
            ),
            (|c| c.output.fault = None, "output.fault"),
            (|c| c.input.gs_base = 0, "input.gs_base"),
            (|c| c.undefined.fp.fsw = 0, "undefined"),
            (|c| c.undefined.mem[0].bytes[1] = 0xf0, "undefined"),
        ] {
            let mut changed = fixture.clone();
            mutate(&mut changed.cases[1]);
            let d = diff(&fixture, &changed);
            assert_eq!(d.len(), 1, "{d:?}");
            assert!(d[0].contains(name), "{d:?}");
        }
    }

    #[test]
    fn rejects_bad_v2_fields() {
        let text = to_json(&sample(), &Header::default());
        for (from, to) in [
            ("\"kind\": \"DE\"", "\"kind\": \"XX\""),
            ("\"st0\": \"0x3fff", "\"st0\": \"0x13fff"),
            ("\"fcw\": \"0x37f\"", "\"fcw\": \"0x37e\""),
            ("\"format\": 2", "\"format\": 1"),
            // A memory mask outside every compared range.
            (
                "\"addr\": \"0x200014\", \"hex\": \"ff0f\"",
                "\"addr\": \"0x200030\", \"hex\": \"ff0f\"",
            ),
            // Or straddling the end of one.
            (
                "\"addr\": \"0x200014\", \"hex\": \"ff0f\"",
                "\"addr\": \"0x200023\", \"hex\": \"ff0f\"",
            ),
        ] {
            assert!(text.contains(from), "{from}");
            let bad = text.replacen(from, to, 1);
            assert!(parse(&bad).is_err(), "accepted {to}");
        }
    }

    #[test]
    fn normalizes_undefined_bits() {
        let mut c = sample().cases[1].clone();
        c.flags_mask = 0x400; // only DF compared
        c.input.rflags = 0x202;
        c.output.rflags = 0x2c7;
        c.input.gprs[2] = 0x1111_1111_2222_2222;
        c.output.gprs[2] = 0x3333_3333_4444_4444;
        c.output.fp.xmm[3] = u128::MAX;
        c.output.fp.fsw = 0x4700 | 0x0800;
        // Input bytes 0x14, 0x15 are 4, 5; the mask is ff 0f.
        c.output.mem[0].bytes[3] = 0xaa;
        c.output.mem[0].bytes[4] = 0xaa;
        c.output.mem[0].bytes[5] = 0xaa;
        c.output.mem[0].bytes[6] = 0xaa;
        c.normalize();
        assert_eq!(&c.output.mem[0].bytes[3..7], &[0xaa, 0x04, 0xa5, 0xaa]);
        assert_eq!(c.output.rflags, 0x202);
        assert_eq!(c.output.gprs[2], 0x3333_3333_2222_2222);
        assert_eq!(c.output.fp.xmm[3], u128::MAX >> 8);
        assert_eq!(c.output.fp.fsw, (c.input.fp.fsw & 0x4700) | 0x0800);
    }

    #[test]
    fn x87_tags() {
        assert_eq!(x87_tag(0x3fff_8000_0000_0000_0000), 0); // 1.0
        assert_eq!(x87_tag(0x8000_0000_0000_0000_0000), 1); // -0.0
        assert_eq!(x87_tag(0x0000_0000_0000_0000_0001), 2); // denormal
        assert_eq!(x87_tag(0x7fff_8000_0000_0000_0000), 2); // infinity
        assert_eq!(x87_tag(0x3fff_4000_0000_0000_0000), 2); // unnormal
        assert_eq!(abridged_tag(0xffff), 0);
        assert_eq!(abridged_tag(0x3fff), 0x80);
        assert_eq!(abridged_tag(0xfff4), 0x03);
        // TOP = 7: ST(0) is R7, ST(1) is R0.
        let mut fp = FpState {
            fsw: 7 << 11,
            ..FpState::DEFAULT
        };
        fp.st[0] = 0x3fff_8000_0000_0000_0000;
        fp.st[1] = 0;
        assert_eq!(fp.physical()[7], fp.st[0]);
        assert_eq!(fp.physical()[0], fp.st[1]);
        assert_eq!(full_tag_word(0x81, &fp), 0x3ffd);
        let r = fp.physical();
        let mut back = FpState {
            fsw: fp.fsw,
            ..FpState::DEFAULT
        };
        back.set_physical(&r);
        assert_eq!(back.st, fp.st);
    }
}
