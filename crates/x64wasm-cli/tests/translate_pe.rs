//! `x64wasm translate-pe` on a minimal synthetic image.

use std::process::Command;

fn put(f: &mut [u8], at: usize, b: &[u8]) {
    f[at..at + b.len()].copy_from_slice(b);
}

/// One `.text` section holding `mov eax, 42; ret` at the entry point.
fn minimal_image() -> Vec<u8> {
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
    put(&mut f, 0x200, &[0xb8, 42, 0, 0, 0, 0xc3]);
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
