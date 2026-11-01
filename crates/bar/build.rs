//! Embeds `app.manifest` as an RT_MANIFEST resource via a hand-written `.res`
//! file, which the linker converts itself (no mt.exe / rc.exe needed).

use std::path::PathBuf;

fn res_header(out: &mut Vec<u8>, data_size: u32, ty: u16, name: u16, flags: u16, lang: u16) {
    out.extend_from_slice(&data_size.to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    out.extend_from_slice(&[0xFF, 0xFF]);
    out.extend_from_slice(&ty.to_le_bytes());
    out.extend_from_slice(&[0xFF, 0xFF]);
    out.extend_from_slice(&name.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&lang.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=app.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest = std::fs::read("app.manifest").expect("read app.manifest");
    let mut res = Vec::with_capacity(manifest.len() + 72);
    res_header(&mut res, 0, 0, 0, 0, 0);
    const RT_MANIFEST: u16 = 24;
    const MOVEABLE_PURE_DISCARDABLE: u16 = 0x1030;
    res_header(&mut res, manifest.len() as u32, RT_MANIFEST, 1, MOVEABLE_PURE_DISCARDABLE, 0x0409);
    res.extend_from_slice(&manifest);
    while res.len() % 4 != 0 {
        res.push(0);
    }
    let path = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("app.res");
    std::fs::write(&path, res).expect("write app.res");
    println!("cargo:rustc-link-arg-bins={}", path.display());
}
