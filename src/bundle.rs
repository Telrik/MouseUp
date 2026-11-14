use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

include!(concat!(env!("OUT_DIR"), "/bundle.rs"));

pub fn prepare(data_dir: &Path) -> io::Result<Option<PathBuf>> {
    if FILES.is_empty() {
        return Ok(None);
    }
    let root = data_dir.join("bundles").join(BUNDLE_ID);
    for (name, packed) in FILES {
        let mut bytes = Vec::new();
        flate2::read::ZlibDecoder::new(*packed).read_to_end(&mut bytes)?;
        let path = root.join(name);
        if fs::read(&path).ok().as_deref() == Some(bytes.as_slice()) {
            continue;
        }
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, &bytes)?;
    }
    Ok(Some(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires MOUSEUP_BUNDLE=1 and compiled helper binaries"]
    fn embedded_files_are_extracted_and_repaired() {
        assert_eq!(FILES.len(), 4 + crate::catalog::SHAPES.len() * 12);
        let dir = std::env::temp_dir().join(format!("mouseup-bundle-test-{}", std::process::id()));
        let root = prepare(&dir).unwrap().unwrap();
        for (name, packed) in FILES {
            let mut bytes = Vec::new();
            flate2::read::ZlibDecoder::new(*packed)
                .read_to_end(&mut bytes)
                .unwrap();
            assert_eq!(fs::read(root.join(name)).unwrap(), bytes);
            if name.starts_with("Cursors/") {
                let source = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("assets/yolomouse")
                    .join(name);
                assert_eq!(bytes, fs::read(source).unwrap(), "cursor changed: {name}");
            }
        }
        let file = root.join(FILES[0].0);
        fs::write(&file, b"damaged").unwrap();
        assert_eq!(prepare(&dir).unwrap().unwrap(), root);
        let mut original = Vec::new();
        flate2::read::ZlibDecoder::new(FILES[0].1)
            .read_to_end(&mut original)
            .unwrap();
        assert_eq!(fs::read(file).unwrap(), original);
        fs::remove_dir_all(dir).unwrap();
    }
}
