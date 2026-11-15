use std::io::Read;

use image::{imageops::FilterType, ImageFormat, RgbaImage};

include!(concat!(env!("OUT_DIR"), "/cursors.rs"));

pub const COLORS: [&str; 12] = [
    "Blue", "DarkBlue", "Green", "Yellow", "Orange", "Red", "Pink", "Purple", "Light", "Silver",
    "Dark", "Inverted",
];

pub fn id(shape: &str, color: &str) -> String {
    format!("@yolo\\{shape}\\{color}.cur")
}

pub fn parts(id: &str) -> Option<(&str, &str)> {
    let (shape, file) = id.strip_prefix("@yolo\\")?.split_once('\\')?;
    let color = file.strip_suffix(".cur")?;
    (crate::catalog::SHAPES.contains(&shape) && COLORS.contains(&color)).then_some((shape, color))
}

pub fn decode(id: &str, size: u32) -> Result<(RgbaImage, (u16, u16)), String> {
    let (shape, color) = parts(id).ok_or("Unknown cursor")?;
    let name = format!("Cursors/{shape}/{color}.cur");
    let (_, packed) = CURSORS
        .iter()
        .find(|(path, _)| *path == name)
        .ok_or("Cursor missing")?;
    let mut data = Vec::new();
    flate2::read::ZlibDecoder::new(*packed)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() < 6 || data[2..4] != [2, 0] {
        return Err("Invalid cursor header".into());
    }
    let count = u16::from_le_bytes([data[4], data[5]]) as usize;
    let mut best: Option<(u32, usize)> = None;
    for i in 0..count {
        let offset = 6 + i * 16;
        let Some(entry) = data.get(offset..offset + 16) else {
            return Err("Invalid cursor directory".into());
        };
        let width = if entry[0] == 0 { 256 } else { entry[0] as u32 };
        let distance = width.abs_diff(size);
        if best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, offset));
        }
    }
    let offset = best.ok_or("Cursor has no images")?.1;
    let entry = &data[offset..offset + 16];
    let source_width = if entry[0] == 0 { 256 } else { entry[0] as u32 };
    let source_height = if entry[1] == 0 { 256 } else { entry[1] as u32 };
    let hot_x = u16::from_le_bytes([entry[4], entry[5]]) as u32;
    let hot_y = u16::from_le_bytes([entry[6], entry[7]]) as u32;
    let length = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
    let start = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
    let frame = data
        .get(start..start.checked_add(length).ok_or("Invalid cursor image")?)
        .ok_or("Invalid cursor image")?;
    // For 32-bit CUR bitmaps, the alpha channel takes precedence over the AND
    // mask. The generic ICO decoder applies both and erases translucent pixels.
    let bitmap = if frame.len() >= 40
        && u32::from_le_bytes(frame[0..4].try_into().unwrap()) == 40
        && i32::from_le_bytes(frame[4..8].try_into().unwrap()) == source_width as i32
        && i32::from_le_bytes(frame[8..12].try_into().unwrap()) == (source_height * 2) as i32
        && u16::from_le_bytes(frame[14..16].try_into().unwrap()) == 32
        && u32::from_le_bytes(frame[16..20].try_into().unwrap()) == 0
    {
        let pixels_len = (source_width * source_height * 4) as usize;
        frame.get(40..40 + pixels_len).and_then(|pixels| {
            if !pixels.chunks_exact(4).any(|p| p[3] != 0) {
                return None;
            }
            let mut image = RgbaImage::new(source_width, source_height);
            for y in 0..source_height {
                for x in 0..source_width {
                    let i = (((source_height - 1 - y) * source_width + x) * 4) as usize;
                    image.put_pixel(
                        x,
                        y,
                        image::Rgba([pixels[i + 2], pixels[i + 1], pixels[i], pixels[i + 3]]),
                    );
                }
            }
            Some(image)
        })
    } else {
        None
    };
    let image = if let Some(image) = bitmap {
        image
    } else {
        let mut icon = vec![0, 0, 1, 0, 1, 0];
        icon.extend_from_slice(entry);
        icon[18..22].copy_from_slice(&22u32.to_le_bytes());
        icon.extend_from_slice(frame);
        image::load_from_memory_with_format(&icon, ImageFormat::Ico)
            .map_err(|e| e.to_string())?
            .to_rgba8()
    };
    let image = image::imageops::resize(&image, size, size, FilterType::Lanczos3);
    Ok((
        image,
        (
            (hot_x * size / source_width) as u16,
            (hot_y * size / source_height) as u16,
        ),
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_original_cursors_decode() {
        for shape in crate::catalog::SHAPES {
            for color in super::COLORS {
                let (image, (x, y)) = super::decode(&super::id(shape, color), 48).unwrap();
                assert_eq!(image.dimensions(), (48, 48));
                assert!(x < 48 && y < 48);
                assert!(image.pixels().any(|p| p[3] != 0));
            }
        }
    }

    #[test]
    fn circle_keeps_its_translucent_fill() {
        let (image, _) = super::decode(&super::id("Circle", "Blue"), 64).unwrap();
        assert!(
            image.get_pixel(24, 24)[3] > 0,
            "circle interior must be visible"
        );
    }
}
