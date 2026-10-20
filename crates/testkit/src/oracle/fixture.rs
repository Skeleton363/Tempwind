//! The L1 fixture format: one JSON file per mnemonic family.
//!
//! ```text
//! {
//!   "header": {...},            CPU model, kernel, date. `record --check` ignores it.
//!   "format": 1,
//!   "family": "add",
//!   "seed": "0x...",
//!   "layout": {...},            CODE_ADDR, DATA_ADDR, ... (must match the oracle constants)
//!   "default_gprs": {...},      input value of every register a case does not list
//!   "cases": [
//!     {"form": ..., "asm": ..., "bytes": "48 01 d8", "flags_mask": "0xcd5",
//!      "input":  {"rflags": ..., "gprs": {only registers != default_gprs},
//!                 "mem": [{"addr", "hex"}, ...]},
//!      "output": {"rip": ..., "rflags": ..., "gprs": {only registers != input},
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
use serde_json::Value;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    /// All 16 registers (the file lists only those that differ from `default_gprs`).
    pub gprs: [u64; 16],
    pub rflags: u64,
    /// Initial contents of the compared ranges, sorted and disjoint.
    pub mem: Vec<MemBlock>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    /// All 16 registers (the file lists only those that differ from the input).
    pub gprs: [u64; 16],
    pub rip: u64,
    pub rflags: u64,
    /// Final contents of every compared range, in `Input::mem` order (the
    /// file lists only the ranges that changed).
    pub mem: Vec<MemBlock>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    pub form: String,
    pub bytes: Vec<u8>,
    pub asm: String,
    /// RFLAGS bits whose output value is architecturally defined and compared.
    pub flags_mask: u64,
    pub input: Input,
    pub output: Output,
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
        let _ = writeln!(
            out,
            "    {{\"form\": {}, \"asm\": {}, \"bytes\": {}, \"flags_mask\": {},",
            json_str(&case.form),
            json_str(&case.asm),
            json_str(&bytes_hex(&case.bytes)),
            json_str(&hex(case.flags_mask))
        );
        let _ = write!(
            out,
            "     \"input\": {{\"rflags\": {}, \"gprs\": {},",
            json_str(&hex(case.input.rflags)),
            gpr_object(&in_gprs)
        );
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

fn parse_u64(text: &str) -> Result<u64> {
    let digits = text
        .strip_prefix("0x")
        .with_context(|| format!("`{text}` is not a 0x-prefixed hex string"))?;
    u64::from_str_radix(digits, 16).with_context(|| format!("bad hex value `{text}`"))
}

fn u64_field(v: &Value, key: &str) -> Result<u64> {
    parse_u64(str_field(v, key)?).with_context(|| format!("field `{key}`"))
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

/// Parses a fixture file. Fails if its format or layout differs from this build's.
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
            Ok(Case {
                form: str_field(c, "form")?.to_string(),
                bytes: parse_bytes(str_field(c, "bytes")?)?,
                asm: str_field(c, "asm")?.to_string(),
                flags_mask: u64_field(c, "flags_mask")?,
                output: Output {
                    gprs: parse_gprs(field(output_v, "gprs")?, &input.gprs)?,
                    rip: u64_field(output_v, "rip")?,
                    rflags: u64_field(output_v, "rflags")?,
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

fn fmt_mem(mem: &[MemBlock]) -> String {
    let parts: Vec<String> = mem
        .iter()
        .map(|b| format!("{:#x}:[{}]", b.addr, mem_hex(&b.bytes)))
        .collect();
    parts.join(" ")
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
            "input.gprs",
            fmt_gprs(&a.input.gprs),
            fmt_gprs(&b.input.gprs),
        );
        field("input.rflags", hex(a.input.rflags), hex(b.input.rflags));
        field("input.mem", fmt_mem(&a.input.mem), fmt_mem(&b.input.mem));
        field(
            "output.gprs",
            fmt_gprs(&a.output.gprs),
            fmt_gprs(&b.output.gprs),
        );
        field("output.rip", hex(a.output.rip), hex(b.output.rip));
        field("output.rflags", hex(a.output.rflags), hex(b.output.rflags));
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
            case.output = Output {
                gprs: [0; 16],
                rip: 0,
                rflags: 0,
                mem: Vec::new(),
            };
        }
        f
    };
    diff(&strip(committed), &strip(generated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
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
        let fixture = Fixture {
            family: "add".into(),
            seed: 42,
            default_gprs: [7; 16],
            cases: vec![Case {
                form: "add_m8_r8".into(),
                bytes: vec![0x00, 0x18],
                asm: "add [rax], bl".into(),
                flags_mask: 0xcd5,
                input: Input {
                    gprs,
                    rflags: 0x202,
                    mem: input_mem,
                },
                output: Output {
                    gprs: out_gprs,
                    rip: CODE_ADDR + 2,
                    rflags: 0x246,
                    mem: output_mem,
                },
            }],
        };
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

        let mut changed = fixture.clone();
        changed.cases[0].output.rflags ^= 1;
        let d = diff(&fixture, &changed);
        assert_eq!(d.len(), 1);
        assert!(d[0].contains("output.rflags"), "{d:?}");
        assert!(diff_inputs(&fixture, &changed).is_empty());
        changed.cases[0].input.mem[0].bytes[3] ^= 0xff;
        assert_eq!(diff_inputs(&fixture, &changed).len(), 1);
    }
}
