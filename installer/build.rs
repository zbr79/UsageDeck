use std::path::PathBuf;

/// Points `USAGEDECK_NSIS_PAYLOAD` (read by `include_bytes!` in src/system.rs) at the NSIS
/// installer to embed. The release workflow sets `USAGEDECK_NSIS_PATH` to the
/// `UsageDeck_<version>_x64-setup.exe` tauri-action built; other builds embed an empty file and
/// the installer reports that it has nothing to install. A release build without the real
/// installer fails here, so it cannot be published by mistake.
fn main() {
    println!("cargo:rerun-if-env-changed=USAGEDECK_NSIS_PATH");
    let payload = match std::env::var_os("USAGEDECK_NSIS_PATH") {
        Some(path) => real_payload(PathBuf::from(path)),
        None => {
            if std::env::var("PROFILE").as_deref() == Ok("release") {
                panic!(
                    "USAGEDECK_NSIS_PATH is not set: a release build of the installer must embed \
                     the NSIS installer (src-tauri/target/release/bundle/nsis/UsageDeck_<version>_x64-setup.exe)"
                );
            }
            let placeholder =
                PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("no-nsis-payload.bin");
            std::fs::write(&placeholder, []).unwrap();
            placeholder
        }
    };
    println!(
        "cargo:rustc-env=USAGEDECK_NSIS_PAYLOAD={}",
        payload.display()
    );

    let windows = tauri_build::WindowsAttributes::new()
        .app_manifest(include_str!("windows-app-manifest.xml"));
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("tauri-build failed");
}

fn real_payload(path: PathBuf) -> PathBuf {
    let path = std::path::absolute(&path).unwrap_or(path);
    println!("cargo:rerun-if-changed={}", path.display());
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("USAGEDECK_NSIS_PATH {}: {e}", path.display()));
    assert!(
        bytes.starts_with(b"MZ"),
        "USAGEDECK_NSIS_PATH {} is not a Windows executable",
        path.display()
    );
    let version = env!("CARGO_PKG_VERSION");
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    assert!(
        name.contains(&format!("_{version}_")),
        "USAGEDECK_NSIS_PATH {name} is not the {version} installer; the installer crate's version \
         must match the app's"
    );
    path
}
