//! `x64wasm translate-pe` on a minimal synthetic image.

use std::process::Command;

fn put(f: &mut [u8], at: usize, b: &[u8]) {
    f[at..at + b.len()].copy_from_slice(b);
}

/// One `.text` section holding `mov eax, 42; ret` at the entry point.
fn minimal_image() -> Vec<u8> {
    image_with_code(&[0xb8, 42, 0, 0, 0, 0xc3])
}

/// One `.text` section (at most 16 code bytes) at the entry point.
fn image_with_code(code: &[u8]) -> Vec<u8> {
    let mut f = vec![0u8; 0x400];
    put(&mut f, 0, b"MZ");
    put(&mut f, 0x3c, &0x40u32.to_le_bytes());
    put(&mut f, 0x40, b"PE\0\0");
    let coff = 0x44;
    put(&mut f, coff, &0x8664u16.to_le_bytes());
    put(&mut f, coff + 2, &1u16.to_le_bytes());
    put(&mut f, coff + 16, &240u16.to_le_bytes());
    put(&mut f, coff + 18, &0x22u16.to_le_bytes());
    let opt = coff + 20;
    put(&mut f, opt, &0x20bu16.to_le_bytes());
    put(&mut f, opt + 16, &0x1000u32.to_le_bytes());
    put(&mut f, opt + 24, &0x1_4000_0000u64.to_le_bytes());
    put(&mut f, opt + 32, &0x1000u32.to_le_bytes());
    put(&mut f, opt + 36, &0x200u32.to_le_bytes());
    put(&mut f, opt + 56, &0x2000u32.to_le_bytes());
    put(&mut f, opt + 60, &0x200u32.to_le_bytes());
    put(&mut f, opt + 68, &3u16.to_le_bytes());
    put(&mut f, opt + 72, &0x10_0000u64.to_le_bytes());
    put(&mut f, opt + 80, &0x1000u64.to_le_bytes());
    put(&mut f, opt + 108, &16u32.to_le_bytes());
    let s = opt + 240;
    put(&mut f, s, b".text");
    put(&mut f, s + 8, &0x10u32.to_le_bytes());
    put(&mut f, s + 12, &0x1000u32.to_le_bytes());
    put(&mut f, s + 16, &0x200u32.to_le_bytes());
    put(&mut f, s + 20, &0x200u32.to_le_bytes());
    put(&mut f, s + 36, &0x6000_0020u32.to_le_bytes());
    put(&mut f, 0x200, code);
    f
}

#[test]
fn writes_module_and_plan() {
    let dir = std::env::temp_dir().join(format!("x64wasm-translate-pe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("tiny.exe");
    std::fs::write(&exe, minimal_image()).unwrap();
    let out_dir = dir.join("out");
    for entries in ["all", "entry"] {
        let out = Command::new(env!("CARGO_BIN_EXE_x64wasm"))
            .args(["translate-pe"])
            .arg(&exe)
            .arg("--out-dir")
            .arg(&out_dir)
            .args(["--entries", entries])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let line = String::from_utf8(out.stdout).unwrap();
        assert!(
            line.starts_with("tiny.exe: 1 functions, 1 blocks, 2 guest instructions, "),
            "{line}"
        );
        assert!(line.trim_end().ends_with(" ms"), "{line}");
        let wasm = std::fs::read(out_dir.join("tiny.wasm")).unwrap();
        wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
            .validate_all(&wasm)
            .unwrap();
        let plan: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out_dir.join("tiny.plan.json")).unwrap())
                .unwrap();
        assert_eq!(plan["file"], "tiny.exe");
        assert_eq!(plan["entryPoint"], "0x140001000");
        assert_eq!(plan["imports"], serde_json::json!([]));
        assert_eq!(plan["tls"], serde_json::Value::Null);
        assert_eq!(plan["hasRelocations"], false);
    }
    // A file that isn't a PE image fails with a message.
    std::fs::write(&exe, b"not a PE").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_x64wasm"))
        .arg("translate-pe")
        .arg(&exe)
        .arg("--out-dir")
        .arg(&out_dir)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("parsing"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The plan's `options` name the options the module was translated with
/// (`runtime.md`, "Load plan"), which the runtime passes on to the
/// run-time translator.
#[test]
fn plan_records_the_options() {
    let dir = std::env::temp_dir().join(format!("x64wasm-plan-options-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("tiny.exe");
    std::fs::write(&exe, minimal_image()).unwrap();
    let cases: [(&[&str], [bool; 5]); 5] = [
        (&[], [false, false, false, false, true]),
        (
            &["--null-check", "--counters"],
            [true, false, true, false, true],
        ),
        (&["--store-check"], [false, true, false, false, true]),
        (&["--sync-all"], [false, false, false, true, true]),
        (&["--no-structured"], [false, false, false, false, false]),
    ];
    for (flags, [null_check, store, counters, sync_all, structured]) in cases {
        let out_dir = dir.join(format!("out{}", flags.join("")));
        let out = Command::new(env!("CARGO_BIN_EXE_x64wasm"))
            .arg("translate-pe")
            .arg(&exe)
            .arg("--out-dir")
            .arg(&out_dir)
            .args(flags)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let plan: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out_dir.join("tiny.plan.json")).unwrap())
                .unwrap();
        assert_eq!(
            plan["options"],
            serde_json::json!({
                "nullCheck": null_check,
                "storeCheck": store,
                "counters": counters,
                "syncAll": sync_all,
                "structured": structured,
            }),
            "{flags:?}"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `--sync-all` and `--no-structured` give the module that translating with
/// `Options::sync_all` / `structured = false` gives (`translator.md`,
/// "Registers and flags", "Jumps and branches").
#[test]
fn reference_mode_flags_set_the_options() {
    use x64wasm::image::{EntrySet, translate};
    use x64wasm::{Options, pe::Pe};
    // mov eax, 42; cmp eax, 42; jne +2; inc eax; ret
    let image = image_with_code(&[0xb8, 42, 0, 0, 0, 0x83, 0xf8, 42, 0x75, 2, 0xff, 0xc0, 0xc3]);
    let dir = std::env::temp_dir().join(format!("x64wasm-reference-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("ref.exe");
    std::fs::write(&exe, &image).unwrap();
    let pe = Pe::parse(&image).unwrap();
    let direct = |opts: Options| translate(&pe, EntrySet::All, &opts).unwrap().wasm;
    let default = direct(Options::default());
    let cases: [(&str, Options); 2] = [
        (
            "--sync-all",
            Options {
                sync_all: true,
                ..Options::default()
            },
        ),
        (
            "--no-structured",
            Options {
                structured: false,
                ..Options::default()
            },
        ),
    ];
    for (flag, opts) in cases {
        let out_dir = dir.join(format!("out{flag}"));
        let out = Command::new(env!("CARGO_BIN_EXE_x64wasm"))
            .arg("translate-pe")
            .arg(&exe)
            .arg("--out-dir")
            .arg(&out_dir)
            .arg(flag)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let wasm = std::fs::read(out_dir.join("ref.wasm")).unwrap();
        let expected = direct(opts);
        assert_ne!(expected, default, "{flag} must change the module");
        assert_eq!(wasm, expected, "{flag}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
