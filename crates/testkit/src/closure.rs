//! The DLL closure of Phase 2 (`testkit closure`, `docs/design/wine-build.md`,
//! "The DLL closure"): the PE modules a run of the root programs may load.
//!
//! Static edges come from the PE tables of each module: imports, delay-load
//! imports and export forwarders; `api-ms-win-*` and `ext-ms-win-*` names are
//! resolved through Wine's `dlls/apisetschema/apisetschema.spec`. Dynamic edges
//! are the modules native Wine logged with `WINEDEBUG=+loaddll`, one trace file
//! per run, named `<root>__<unit>.log`. The union is expanded statically again,
//! because a module loaded at run time has imports of its own.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use x64wasm::pe::Pe;

/// Where the PE modules are inside an install tree.
const PE_DIR: &str = "lib/wine/x86_64-windows";

/// Arguments of `testkit closure`.
#[derive(clap::Args, Debug)]
pub struct Args {
    /// Root of the unpacked `wine-pe-x86_64.tar.zst` (holds `lib/wine/x86_64-windows`).
    #[arg(long)]
    pub tree: PathBuf,
    /// A root program (repeatable). Its file name names the root.
    #[arg(long = "root", required = true)]
    pub roots: Vec<PathBuf>,
    /// Wine's `dlls/apisetschema/apisetschema.spec`.
    #[arg(long)]
    pub apiset: PathBuf,
    /// Directory of `+loaddll` traces, `<root>__<unit>.log`.
    #[arg(long)]
    pub traces: Option<PathBuf>,
    /// The release tag, `wine-<sha7>`.
    #[arg(long)]
    pub wine: String,
    /// A data file of the Unix side, relative to the install prefix (repeatable).
    #[arg(long = "data-file")]
    pub data_files: Vec<String>,
    /// Text for the "note" field of `data_files`.
    #[arg(long)]
    pub note: Option<String>,
    /// Output JSON.
    #[arg(long)]
    pub out: PathBuf,
}

/// `apiset <name> = <host>` lines, keyed by the name without its version.
pub type Apiset = BTreeMap<String, BTreeSet<String>>;

/// `api-ms-win-core-foo-l1-1-3` -> `api-ms-win-core-foo-l1-1`: Wine matches
/// contract names without the last version number.
fn apiset_key(name: &str) -> String {
    let name = name.strip_suffix(".dll").unwrap_or(name);
    match name.rsplit_once('-') {
        Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => {
            head.to_string()
        }
        _ => name.to_string(),
    }
}

fn is_apiset(name: &str) -> bool {
    name.starts_with("api-ms-win-") || name.starts_with("ext-ms-win-")
}

pub fn parse_apiset(text: &str) -> Apiset {
    let mut map = Apiset::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("apiset ") else {
            continue;
        };
        let Some((name, host)) = rest.split_once('=') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let Some(host) = host.split_whitespace().next() else {
            continue;
        };
        map.entry(apiset_key(&name))
            .or_default()
            .insert(host.to_ascii_lowercase());
    }
    map
}

/// Module base names from `+loaddll` lines such as
/// `0024:trace:loaddll:build_module Loaded L"C:\\windows\\system32\\kernelbase.dll" at 00007FF…: builtin`.
pub fn parse_loaddll(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let Some(at) = line.find("Loaded ") else {
            continue;
        };
        if !line[..at].contains("loaddll") {
            continue;
        }
        let rest = &line[at + "Loaded ".len()..];
        let rest = rest.strip_prefix('L').unwrap_or(rest);
        let Some(rest) = rest.strip_prefix('"') else {
            continue;
        };
        let Some(end) = rest.find('"') else { continue };
        let path = &rest[..end];
        let base = path.rsplit(['\\', '/']).next().unwrap_or(path);
        if !base.is_empty() {
            out.insert(base.to_ascii_lowercase());
        }
    }
    out
}

struct Info {
    path: String,
    base: u64,
    size: u64,
    deps: Vec<String>,
}

fn load_info(file: &Path, path: String) -> Result<Info> {
    let data = fs::read(file).with_context(|| format!("read {}", file.display()))?;
    let pe = Pe::parse(&data).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
    let mut deps: Vec<String> = Vec::new();
    for d in pe
        .imports()
        .map_err(|e| anyhow::anyhow!("{}: imports: {e}", file.display()))?
    {
        deps.push(d.name);
    }
    for d in pe
        .delay_imports()
        .map_err(|e| anyhow::anyhow!("{}: delay imports: {e}", file.display()))?
    {
        deps.push(d.name);
    }
    if let Some(ex) = pe
        .exports()
        .map_err(|e| anyhow::anyhow!("{}: exports: {e}", file.display()))?
    {
        for f in ex.exports.iter().filter_map(|e| e.forwarder.as_deref()) {
            if let Some((dll, _)) = f.rsplit_once('.') {
                deps.push(dll.to_string());
            }
        }
    }
    deps.sort_by_key(|d| d.to_ascii_lowercase());
    deps.dedup_by_key(|d| d.to_ascii_lowercase());
    Ok(Info {
        path,
        base: pe.image_base,
        size: pe.size_of_image as u64,
        deps,
    })
}

struct Closure {
    /// Everything found in the tree, lowercase file name -> info.
    infos: BTreeMap<String, Info>,
    apiset: Apiset,
    /// Names that are neither in the tree nor an apiset with a host: name -> importers.
    unresolved: BTreeMap<String, BTreeSet<String>>,
}

impl Closure {
    /// Tree module names a dependency name stands for.
    fn resolve(&mut self, from: &str, dep: &str) -> Vec<String> {
        let dep = dep.to_ascii_lowercase();
        if is_apiset(&dep)
            && let Some(hosts) = self.apiset.get(&apiset_key(&dep))
        {
            return hosts.iter().cloned().collect();
        }
        for cand in [dep.clone(), format!("{dep}.dll")] {
            if self.infos.contains_key(&cand) {
                return vec![cand];
            }
        }
        self.unresolved
            .entry(dep)
            .or_default()
            .insert(from.to_string());
        Vec::new()
    }

    /// The modules reachable from `seeds` (name, via) by static edges.
    fn expand(
        &mut self,
        seeds: &[(String, &'static str)],
    ) -> BTreeMap<String, BTreeSet<&'static str>> {
        let mut vias: BTreeMap<String, BTreeSet<&'static str>> = BTreeMap::new();
        let mut queue: VecDeque<String> = VecDeque::new();
        let mut expanded: BTreeSet<String> = BTreeSet::new();
        for (name, via) in seeds {
            if !self.infos.contains_key(name) {
                self.unresolved
                    .entry(name.clone())
                    .or_default()
                    .insert(format!("({via})"));
                continue;
            }
            vias.entry(name.clone()).or_default().insert(via);
            queue.push_back(name.clone());
        }
        while let Some(name) = queue.pop_front() {
            if !expanded.insert(name.clone()) {
                continue;
            }
            let deps = self.infos[&name].deps.clone();
            for dep in deps {
                for host in self.resolve(&name, &dep) {
                    vias.entry(host.clone()).or_default().insert("static");
                    queue.push_back(host);
                }
            }
        }
        vias
    }
}

/// A root's `path` in `closure.json`: relative to the tree when the root is
/// inside it (`tests/kernel32_test.exe`), with `/` separators; otherwise as
/// given (`wine-build.md`, "The DLL closure").
fn root_path(tree: &Path, root: &Path) -> String {
    let rel = root
        .strip_prefix(tree)
        .ok()
        .map(Path::to_path_buf)
        .or_else(|| {
            let (tree, root) = (tree.canonicalize().ok()?, root.canonicalize().ok()?);
            root.strip_prefix(tree).ok().map(Path::to_path_buf)
        });
    match rel {
        Some(rel) => rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        None => root.to_string_lossy().into_owned(),
    }
}

fn hex(v: u64) -> String {
    format!("{v:#x}")
}

pub fn run(args: &Args) -> Result<()> {
    let pe_dir = args.tree.join(PE_DIR);
    let mut infos = BTreeMap::new();
    for entry in fs::read_dir(&pe_dir).with_context(|| format!("read {}", pe_dir.display()))? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        let rel = format!("{PE_DIR}/{}", entry.file_name().to_string_lossy());
        match load_info(&entry.path(), rel) {
            Ok(info) => {
                infos.insert(name, info);
            }
            // Not a PE image (a data file next to the modules): not a module.
            Err(e) => eprintln!("closure: skipping {name}: {e}"),
        }
    }
    if infos.is_empty() {
        bail!("no PE modules in {}", pe_dir.display());
    }
    let apiset = parse_apiset(
        &fs::read_to_string(&args.apiset)
            .with_context(|| format!("read {}", args.apiset.display()))?,
    );
    if apiset.is_empty() {
        bail!("no apiset lines in {}", args.apiset.display());
    }
    let mut cl = Closure {
        infos,
        apiset,
        unresolved: BTreeMap::new(),
    };

    // Roots join the table under their own names.
    let mut root_names = Vec::new();
    for r in &args.roots {
        let name = r
            .file_name()
            .context("root without a file name")?
            .to_string_lossy()
            .to_ascii_lowercase();
        let info = load_info(r, root_path(&args.tree, r))?;
        cl.infos.insert(name.clone(), info);
        root_names.push(name);
    }

    // Traces: `<root>__<unit>.log`.
    let mut traced: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut trace_files = 0usize;
    if let Some(dir) = &args.traces {
        for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
            let entry = entry?;
            let fname = entry.file_name().to_string_lossy().into_owned();
            let Some(stem) = fname.strip_suffix(".log") else {
                continue;
            };
            let root = stem.split("__").next().unwrap_or(stem).to_ascii_lowercase();
            let text = fs::read(entry.path())?;
            traced
                .entry(root)
                .or_default()
                .extend(parse_loaddll(&String::from_utf8_lossy(&text)));
            trace_files += 1;
        }
    }
    for r in traced.keys() {
        if !root_names.contains(r) {
            eprintln!("closure: traces for {r}, which is not a root; ignored");
        }
    }

    // Per root: the root, apisetschema.dll, its traced modules, then static edges.
    let mut modules: BTreeMap<String, (BTreeSet<&'static str>, BTreeSet<String>)> = BTreeMap::new();
    for root in &root_names {
        let mut seeds = vec![
            (root.clone(), "root"),
            ("apisetschema.dll".to_string(), "always"),
        ];
        for m in traced.get(root).into_iter().flatten() {
            // The program itself and its own copies are not tree modules.
            if m != root && cl.infos.contains_key(m) && !root_names.contains(m) {
                seeds.push((m.clone(), "loaddll"));
            } else if m != root {
                cl.unresolved
                    .entry(m.clone())
                    .or_default()
                    .insert(format!("(loaddll {root})"));
            }
        }
        for (name, vias) in cl.expand(&seeds) {
            let e = modules.entry(name).or_default();
            e.0.extend(vias);
            e.1.insert(root.clone());
        }
    }

    // Layout: the roots are never loaded together, so overlaps are between DLLs
    // (and between a root and the DLLs it loads).
    let mut list = Vec::new();
    let mut ranges: Vec<(String, u64, u64)> = Vec::new();
    let mut root_overlaps: Vec<(String, String)> = Vec::new();
    for (name, (vias, roots)) in &modules {
        let info = &cl.infos[name];
        let is_root = root_names.contains(name);
        let mut v: Vec<&str> = vias.iter().copied().collect();
        v.sort_by_key(|x| {
            ["root", "always", "static", "loaddll"]
                .iter()
                .position(|y| y == x)
        });
        list.push(json!({
            "name": name, "path": info.path, "base": hex(info.base), "size": hex(info.size),
            "via": v, "roots": roots,
        }));
        if !is_root {
            ranges.push((name.clone(), info.base, info.base + info.size));
        }
    }
    let mut overlaps = Vec::new();
    for (i, a) in ranges.iter().enumerate() {
        for b in &ranges[i + 1..] {
            if a.1 < b.2 && b.1 < a.2 {
                overlaps.push([a.0.clone(), b.0.clone()]);
            }
        }
    }
    for root in &root_names {
        let ri = &cl.infos[root];
        for (name, (_, roots)) in &modules {
            if root_names.contains(name) || !roots.contains(root) {
                continue;
            }
            let di = &cl.infos[name];
            if ri.base < di.base + di.size && di.base < ri.base + ri.size {
                root_overlaps.push((root.clone(), name.clone()));
            }
        }
    }
    let highest_end = modules
        .keys()
        .map(|n| cl.infos[n].base + cl.infos[n].size)
        .max()
        .unwrap_or(0);
    let lowest_base = modules.keys().map(|n| cl.infos[n].base).min().unwrap_or(0);
    let sum_sizes: u64 = modules.keys().map(|n| cl.infos[n].size).sum();
    let memory_bytes = highest_end.div_ceil(65536) * 65536;

    let mut out = json!({
        "schema": 1,
        "wine": args.wine,
        "modules": list,
        "overlaps": overlaps,
        "root_overlaps": root_overlaps,
        "highest_end": hex(highest_end),
        "memory_bytes": memory_bytes,
        "lowest_base": hex(lowest_base),
        "sum_of_sizes": sum_sizes,
        "memory_bytes_definition": "highest_end rounded up to 64 KiB: guest memory from address 0 with every module at its preferred base",
        "unresolved": cl.unresolved.iter()
            .map(|(n, f)| json!({"name": n, "from": f.iter().take(8).collect::<Vec<_>>()}))
            .collect::<Vec<Value>>(),
        "trace_files": trace_files,
        "data_files": args.data_files,
    });
    if let Some(note) = &args.note {
        out["data_files_note"] = json!(note);
    }
    fs::write(&args.out, serde_json::to_string_pretty(&out)? + "\n")?;
    println!(
        "closure: {} modules ({} roots), {} overlaps, highest_end {}, memory_bytes {}, {} unresolved",
        modules.len(),
        root_names.len(),
        overlaps.len(),
        hex(highest_end),
        memory_bytes,
        cl.unresolved.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apiset_names_match_without_the_last_version() {
        let m = parse_apiset(
            "# comment\napiset api-ms-win-core-foo-l1-1-0 = kernelbase.dll\n\
             apiset api-ms-win-core-foo-l1-1-2 = kernel32.dll\napiset ext-ms-win-x-l1-1-0 = Ntdll.dll\n",
        );
        assert_eq!(
            apiset_key("api-ms-win-core-foo-l1-1-7.dll"),
            "api-ms-win-core-foo-l1-1"
        );
        let hosts = &m["api-ms-win-core-foo-l1-1"];
        assert!(hosts.contains("kernelbase.dll") && hosts.contains("kernel32.dll"));
        assert!(m["ext-ms-win-x-l1-1"].contains("ntdll.dll"));
    }

    #[test]
    fn root_paths_are_tree_relative() {
        let tree = Path::new("tree");
        assert_eq!(
            root_path(tree, Path::new("tree/tests/kernel32_test.exe")),
            "tests/kernel32_test.exe"
        );
        assert_eq!(
            root_path(Path::new("/x/tree/"), Path::new("/x/tree/tests/a.exe")),
            "tests/a.exe"
        );
        // Outside the tree: as given.
        assert_eq!(
            root_path(tree, Path::new("roots/basic.exe")),
            "roots/basic.exe"
        );
        // The tree named another way (`..`) is matched through the
        // canonical paths.
        let dir = std::env::temp_dir().join(format!("closure-roots-{}", std::process::id()));
        fs::create_dir_all(dir.join("tests")).unwrap();
        fs::write(dir.join("tests/b.exe"), b"").unwrap();
        assert_eq!(
            root_path(&dir.join("tests/.."), &dir.join("tests/b.exe")),
            "tests/b.exe"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn loaddll_lines() {
        let t = "0024:trace:loaddll:build_module Loaded L\"C:\\\\windows\\\\system32\\\\KernelBase.dll\" at 00007FF: builtin\n\
                 0024:trace:loaddll:build_module Loaded L\"C:\\\\x\\\\hello.exe\" at 1: native\n\
                 0024:trace:other:foo Loaded L\"no.dll\"\nnoise\n";
        let s = parse_loaddll(t);
        assert_eq!(
            s.into_iter().collect::<Vec<_>>(),
            ["hello.exe", "kernelbase.dll"]
        );
    }
}
