//! Windows resource scriptless icon embedder.
//!
//! Packages `data/icons/hicolor/256x256/apps/co.hyprlab.Hylki.png` into a standard
//! Win32 binary `.res` file (Vista+ ICO supports embedded PNG directly) and passes
//! it to MSVC `link.exe` without requiring `rc.exe` or extra build dependencies.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let icon_png_path = manifest_dir
        .join("..")
        .join("..")
        .join("data")
        .join("icons")
        .join("hicolor")
        .join("256x256")
        .join("apps")
        .join("co.hyprlab.Hylki.png");

    println!("cargo:rerun-if-changed={}", icon_png_path.display());

    let Ok(png_bytes) = fs::read(&icon_png_path) else {
        println!(
            "cargo:warning=Hylki icon PNG not found at {}, skipping exe icon embedding",
            icon_png_path.display()
        );
        return;
    };

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let res_path = out_dir.join("hylki-icon.res");

    let mut res = Vec::with_capacity(png_bytes.len() + 256);
    // 1. Standard 32-byte Win32 .res empty sentinel header.
    write_res_entry(&mut res, 0, 0, 0, &[]);
    // 2. RT_ICON (3), ID = 1: raw 256x256 PNG bytes.
    write_res_entry(&mut res, 3, 1, 0x0409, &png_bytes);
    // 3. RT_GROUP_ICON (14), ID = 1: GRPICONDIR + 1 GRPICONDIRENTRY pointing to icon ID 1.
    let mut group = Vec::with_capacity(20);
    group.extend_from_slice(&0u16.to_le_bytes()); // idReserved
    group.extend_from_slice(&1u16.to_le_bytes()); // idType (1 = ICO)
    group.extend_from_slice(&1u16.to_le_bytes()); // idCount
    group.push(0); // bWidth (0 means 256)
    group.push(0); // bHeight (0 means 256)
    group.push(0); // bColorCount
    group.push(0); // bReserved
    group.extend_from_slice(&1u16.to_le_bytes()); // wPlanes
    group.extend_from_slice(&32u16.to_le_bytes()); // wBitCount
    group.extend_from_slice(&(png_bytes.len() as u32).to_le_bytes()); // dwBytesInRes
    group.extend_from_slice(&1u16.to_le_bytes()); // nId (RT_ICON #1)
    write_res_entry(&mut res, 14, 1, 0x0409, &group);

    fs::write(&res_path, res).expect("failed to write hylki-icon.res");
    println!("cargo:rustc-link-arg={}", res_path.display());
}

fn write_res_entry(buf: &mut Vec<u8>, res_type: u16, res_id: u16, lang_id: u16, data: &[u8]) {
    const HEADER_SIZE: u32 = 32;
    buf.extend_from_slice(&(data.len() as u32).to_le_bytes()); // DataSize
    buf.extend_from_slice(&HEADER_SIZE.to_le_bytes()); // HeaderSize
    buf.extend_from_slice(&0xFFFFu16.to_le_bytes()); // Type ordinal marker
    buf.extend_from_slice(&res_type.to_le_bytes()); // Type ID
    buf.extend_from_slice(&0xFFFFu16.to_le_bytes()); // Name ordinal marker
    buf.extend_from_slice(&res_id.to_le_bytes()); // Name ID
    buf.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    buf.extend_from_slice(&0x1030u16.to_le_bytes()); // MemoryFlags (MOVEABLE | PURE | DISCARDABLE)
    buf.extend_from_slice(&lang_id.to_le_bytes()); // LanguageId
    buf.extend_from_slice(&0u32.to_le_bytes()); // Version
    buf.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    buf.extend_from_slice(data);
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}
