use std::collections::BTreeMap;
use x64wasm::{Options, image::{self, EntrySet}, pe::Pe};
fn main() {
    let data = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let pe = Pe::parse(&data).unwrap();
    let t = image::translate(&pe, EntrySet::All, &Options::default()).unwrap();
    let mut m: BTreeMap<String, usize> = BTreeMap::new();
    for &a in &t.unsupported {
        let rva = (a - pe.image_base) as u32;
        let b = pe.read_rva(rva, 15).or_else(|_| pe.read_rva(rva, 1)).unwrap();
        let mut d = iced_x86::Decoder::with_ip(64, b, a, 0);
        let i = d.decode();
        *m.entry(format!("{:?}", i.mnemonic())).or_default() += 1;
    }
    let mut v: Vec<_> = m.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, n) in v { print!("{k} {n}, "); }
    println!();
}
