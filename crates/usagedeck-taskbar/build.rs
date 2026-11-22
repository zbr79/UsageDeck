use std::path::PathBuf;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RES_HEADER_SIZE: u32 = 32;
/// `MOVEABLE (0x0010) | PURE (0x0020) | DISCARDABLE (0x1000)`
const RES_MEMORY_FLAGS: u16 = 0x1030;
/// `MAKELANGID(LANG_ENGLISH, SUBLANG_ENGLISH_US)`
const RES_LANG_EN_US: u16 = 0x0409;

/// Embeds `assets/icon.ico` as Windows PE `RT_ICON` / `RT_GROUP_ICON` resources into
/// `usagedeck.exe` on MSVC Windows builds using a pure-Rust `.res` generator (no external
/// crates and no `rc.exe` PATH dependency).
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        let manifest_dir = PathBuf::from(
            std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by Cargo"),
        );
        let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));

        let ico_path = manifest_dir.join("assets").join("icon.ico");
        let ico_bytes = std::fs::read(&ico_path).unwrap_or_else(|e| {
            panic!("failed to read app icon {}: {e}", ico_path.display());
        });
        let res_bytes = build_icon_res(&ico_bytes);

        let res_path = out_dir.join("app_icon.res");
        let res_path = std::path::absolute(&res_path).unwrap_or(res_path);
        std::fs::write(&res_path, &res_bytes).unwrap_or_else(|e| {
            panic!(
                "failed to write Win32 icon resource {}: {e}",
                res_path.display()
            );
        });

        println!("cargo:rustc-link-arg-bin=usagedeck={}", res_path.display());
    }
}

/// Converts a `.ico` file (`ICONDIR` + `ICONDIRENTRY[]` + image payloads) into a 32-bit
/// Windows `.res` binary file containing `RT_ICON` (IDs `1..=N`) and `RT_GROUP_ICON` (ID `1`).
fn build_icon_res(ico: &[u8]) -> Vec<u8> {
    assert!(
        ico.len() >= 6,
        "icon.ico is too short to contain an ICONDIR header"
    );
    let reserved = u16::from_le_bytes([ico[0], ico[1]]);
    let image_type = u16::from_le_bytes([ico[2], ico[3]]);
    let count = u16::from_le_bytes([ico[4], ico[5]]);
    assert_eq!(reserved, 0, "icon.ico ICONDIR.idReserved must be 0");
    assert_eq!(image_type, 1, "icon.ico ICONDIR.idType must be 1 (icon)");
    assert!(count > 0, "icon.ico must contain at least one icon image");

    let dir_end = 6 + usize::from(count) * 16;
    assert!(
        ico.len() >= dir_end,
        "icon.ico is truncated inside ICONDIRENTRY table (len={}, needed>={dir_end})",
        ico.len()
    );

    let mut res = Vec::with_capacity(ico.len() + 512);
    // Standard 32-byte Win32 .res sentinel (empty null resource header).
    res.extend_from_slice(&0u32.to_le_bytes()); // DataSize = 0
    res.extend_from_slice(&RES_HEADER_SIZE.to_le_bytes()); // HeaderSize = 32
    res.extend_from_slice(&0xFFFFu16.to_le_bytes()); // Type ordinal prefix
    res.extend_from_slice(&0u16.to_le_bytes()); // Type = 0
    res.extend_from_slice(&0xFFFFu16.to_le_bytes()); // Name ordinal prefix
    res.extend_from_slice(&0u16.to_le_bytes()); // Name = 0
    res.extend_from_slice(&0u32.to_le_bytes()); // DataVersion = 0
    res.extend_from_slice(&0u16.to_le_bytes()); // MemoryFlags = 0
    res.extend_from_slice(&0u16.to_le_bytes()); // LanguageId = 0
    res.extend_from_slice(&0u32.to_le_bytes()); // Version = 0
    res.extend_from_slice(&0u32.to_le_bytes()); // Characteristics = 0

    // GRPICONDIR header (6 bytes) + N * GRPICONDIRENTRY (14 bytes each).
    let mut group_dir = Vec::with_capacity(6 + usize::from(count) * 14);
    group_dir.extend_from_slice(&0u16.to_le_bytes()); // idReserved = 0
    group_dir.extend_from_slice(&1u16.to_le_bytes()); // idType = 1 (icon)
    group_dir.extend_from_slice(&count.to_le_bytes()); // idCount = N

    for index in 0..count {
        let entry_offset = 6 + usize::from(index) * 16;
        let entry = &ico[entry_offset..entry_offset + 16];
        let img_size = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
        let img_offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
        let img_end = img_offset
            .checked_add(img_size)
            .expect("icon.ico image offset + size overflowed");
        assert!(
            img_size > 0 && img_end <= ico.len(),
            "icon.ico entry {} image slice ({img_offset}..{img_end}) is out of bounds (len={})",
            index + 1,
            ico.len()
        );

        let icon_id = index + 1;
        push_res_record(&mut res, RT_ICON, icon_id, &ico[img_offset..img_end]);

        // GRPICONDIRENTRY copies the first 12 bytes of ICONDIRENTRY verbatim, then appends nId.
        group_dir.extend_from_slice(&entry[..12]);
        group_dir.extend_from_slice(&icon_id.to_le_bytes());
    }

    push_res_record(&mut res, RT_GROUP_ICON, 1, &group_dir);
    res
}

/// Appends one 32-bit Win32 `.res` resource record (32-byte ordinal header + payload +
/// DWORD alignment padding).
fn push_res_record(out: &mut Vec<u8>, res_type: u16, res_name: u16, data: &[u8]) {
    let data_size = u32::try_from(data.len()).expect("resource payload exceeds u32::MAX");
    out.extend_from_slice(&data_size.to_le_bytes()); // DataSize
    out.extend_from_slice(&RES_HEADER_SIZE.to_le_bytes()); // HeaderSize = 32
    out.extend_from_slice(&0xFFFFu16.to_le_bytes()); // Type ordinal prefix
    out.extend_from_slice(&res_type.to_le_bytes()); // Type ID
    out.extend_from_slice(&0xFFFFu16.to_le_bytes()); // Name ordinal prefix
    out.extend_from_slice(&res_name.to_le_bytes()); // Name ID
    out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    out.extend_from_slice(&RES_MEMORY_FLAGS.to_le_bytes()); // MemoryFlags
    out.extend_from_slice(&RES_LANG_EN_US.to_le_bytes()); // LanguageId
    out.extend_from_slice(&0u32.to_le_bytes()); // Version
    out.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    out.extend_from_slice(data);

    let pad = (4 - (data.len() % 4)) % 4;
    out.resize(out.len() + pad, 0);
}
