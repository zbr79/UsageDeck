use std::path::PathBuf;

/// Embeds the compiled `usagedeck.exe` pointed to by `USAGEDECK_EXE_PATH` into
/// `OUT_DIR/usagedeck.bin`.
///
/// - In debug/test builds, if `USAGEDECK_EXE_PATH` is unset or empty, writes an empty
///   `usagedeck.bin` so `cargo test` and local checks work without a prebuilt binary.
/// - In release builds (`PROFILE=release`), panics with a clear error if `USAGEDECK_EXE_PATH`
///   is missing, empty, unreadable, or not a valid Windows PE executable (`MZ` header).
/// - On MSVC Windows builds, embeds `windows-app-manifest.xml` (`asInvoker`, `PerMonitorV2`)
///   so Windows' Installer Detection heuristic never prompts for UAC elevation on `*Setup*.exe`.
fn main() {
    println!("cargo:rerun-if-env-changed=USAGEDECK_EXE_PATH");
    println!("cargo:rerun-if-changed=windows-app-manifest.xml");

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let out_bin = out_dir.join("usagedeck.bin");

    let exe_path = std::env::var_os("USAGEDECK_EXE_PATH").filter(|v| !v.is_empty());
    match exe_path {
        Some(raw_path) => {
            let path = PathBuf::from(raw_path);
            let path = std::path::absolute(&path).unwrap_or(path);
            println!("cargo:rerun-if-changed={}", path.display());

            let bytes = std::fs::read(&path).unwrap_or_else(|e| {
                panic!(
                    "USAGEDECK_EXE_PATH ({}) could not be read: {e}",
                    path.display()
                )
            });
            assert!(
                !bytes.is_empty(),
                "USAGEDECK_EXE_PATH ({}) is empty",
                path.display()
            );
            assert!(
                bytes.starts_with(b"MZ"),
                "USAGEDECK_EXE_PATH ({}) is not a valid Windows executable (missing MZ header)",
                path.display()
            );

            std::fs::write(&out_bin, &bytes).unwrap_or_else(|e| {
                panic!(
                    "failed to write embedded payload {}: {e}",
                    out_bin.display()
                )
            });
        }
        None => {
            if std::env::var("PROFILE").as_deref() == Ok("release") {
                panic!(
                    "USAGEDECK_EXE_PATH is missing or empty: a release build of UsageDeck-Setup \
                     must embed the compiled usagedeck.exe (set USAGEDECK_EXE_PATH to the release \
                     usagedeck.exe binary)"
                );
            }
            std::fs::write(&out_bin, []).unwrap_or_else(|e| {
                panic!(
                    "failed to write empty debug placeholder {}: {e}",
                    out_bin.display()
                )
            });
        }
    }

    // Embed the asInvoker + PerMonitorV2 manifest natively via MSVC link.exe (zero external crates).
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        let manifest_dir = PathBuf::from(
            std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by Cargo"),
        );
        let manifest = manifest_dir.join("windows-app-manifest.xml");
        let manifest = std::path::absolute(&manifest).unwrap_or(manifest);
        println!("cargo:rustc-link-arg-bin=UsageDeck-Setup=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bin=UsageDeck-Setup=/MANIFESTUAC:NO");
        println!(
            "cargo:rustc-link-arg-bin=UsageDeck-Setup=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}
