use std::{env, fs, io::Write, path::PathBuf};
#[path = "../src/catalog.rs"]
mod catalog;

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("..");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut entries = String::new();
    let mut index = 0;
    for shape in catalog::SHAPES {
        for color in [
            "Blue", "DarkBlue", "Green", "Yellow", "Orange", "Red", "Pink", "Purple", "Light",
            "Silver", "Dark", "Inverted",
        ] {
            let name = format!("Cursors/{shape}/{color}.cur");
            let source = root.join("assets/yolomouse").join(&name);
            println!("cargo:rerun-if-changed={}", source.display());
            let bytes = fs::read(&source)
                .unwrap_or_else(|e| panic!("Cannot bundle {}: {e}", source.display()));
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder.write_all(&bytes).unwrap();
            let packed = out.join(format!("cursor-{index}.z"));
            fs::write(&packed, encoder.finish().unwrap()).unwrap();
            entries.push_str(&format!(
                "({name:?}, include_bytes!({:?}) as &[u8]),\n",
                packed.to_string_lossy()
            ));
            index += 1;
        }
    }
    fs::write(
        out.join("cursors.rs"),
        format!("const CURSORS: &[(&str, &[u8])] = &[{entries}];\n"),
    )
    .unwrap();
}
