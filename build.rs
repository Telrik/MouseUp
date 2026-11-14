use std::{env, fs, io::Write, path::PathBuf, process::Command};
#[path = "src/catalog.rs"]
mod catalog;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
        let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
        let icon = root.join("assets/brand.ico");
        println!("cargo:rerun-if-changed={}", icon.display());
        let rc = out.join("icon.rc");
        let res = out.join("icon.res");
        fs::write(
            &rc,
            format!(
                "1 ICON \"{}\"\n",
                icon.to_string_lossy().replace('\\', "\\\\")
            ),
        )
        .unwrap();
        let status = Command::new("rc")
            .arg("/nologo")
            .arg(format!("/fo{}", res.display()))
            .arg(&rc)
            .status()
            .expect("Windows SDK resource compiler (rc.exe) is required");
        assert!(status.success(), "Could not compile application icon");
        println!("cargo:rustc-link-arg-bin=mouseup={}", res.display());
    }
    println!("cargo:rerun-if-env-changed=MOUSEUP_BUNDLE");
    let mut files = Vec::new();
    if env::var_os("MOUSEUP_BUNDLE").is_some() {
        let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
        for (name, source) in [
            (
                "mouseup-hook-host64.exe",
                "target/x86_64-pc-windows-msvc/release/mouseup-hook-host.exe",
            ),
            (
                "mouseup-hook64.dll",
                "target/x86_64-pc-windows-msvc/release/mouseup_hook.dll",
            ),
            (
                "mouseup-hook-host32.exe",
                "target/i686-pc-windows-msvc/release/mouseup-hook-host.exe",
            ),
            (
                "mouseup-hook32.dll",
                "target/i686-pc-windows-msvc/release/mouseup_hook.dll",
            ),
        ] {
            files.push((name.to_owned(), root.join(source)));
        }
        for shape in catalog::SHAPES {
            for color in [
                "Blue", "DarkBlue", "Green", "Yellow", "Orange", "Red", "Pink", "Purple", "Light",
                "Silver", "Dark", "Inverted",
            ] {
                let name = format!("Cursors/{shape}/{color}.cur");
                let source = root.join("assets/yolomouse").join(&name);
                files.push((name, source));
            }
        }
    }
    let mut hash = 0xcbf29ce484222325u64;
    let mut entries = String::new();
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    for (index, (name, path)) in files.iter().enumerate() {
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes =
            fs::read(path).unwrap_or_else(|e| panic!("Cannot bundle {}: {e}", path.display()));
        for byte in name.bytes().chain(bytes.iter().copied()) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(&bytes).unwrap();
        let packed = out.join(format!("bundle-{index}.z"));
        fs::write(&packed, encoder.finish().unwrap()).unwrap();
        entries.push_str(&format!(
            "({name:?}, include_bytes!({:?}) as &[u8]),\n",
            packed.to_string_lossy()
        ));
    }
    let generated = format!(
        "const BUNDLE_ID: &str = \"{hash:016x}\";\nconst FILES: &[(&str, &[u8])] = &[{entries}];\n"
    );
    fs::write(out.join("bundle.rs"), generated).unwrap();
}
