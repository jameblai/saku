//! Vision helpers: resize/compress images before Provider send.

use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};
use thiserror::Error;

/// Max longest edge in pixels (code default).
pub const MAX_IMAGE_EDGE: u32 = 2048;
/// Target max encoded bytes after compression.
pub const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum ImageError {
    #[error("image decode: {0}")]
    Decode(String),
    #[error("image encode: {0}")]
    Encode(String),
}

pub fn is_image_path(path: &std::path::Path) -> bool {
    mime_guess::from_path(path)
        .first()
        .is_some_and(|m| m.type_() == mime_guess::mime::IMAGE)
}

pub fn is_image_mime(mime: &str) -> bool {
    mime.starts_with("image/")
}

/// Decode, downscale, and re-encode as JPEG (or keep PNG for transparency-ish cases as JPEG).
pub fn resize_for_provider(bytes: &[u8], mime_hint: Option<&str>) -> Result<(Vec<u8>, String), ImageError> {
    let img = image::load_from_memory(bytes).map_err(|e| ImageError::Decode(e.to_string()))?;
    let resized = downscale(img, MAX_IMAGE_EDGE);
    let mut out = Vec::new();
    let format = choose_format(mime_hint);
    resized
        .write_to(&mut std::io::Cursor::new(&mut out), format)
        .map_err(|e| ImageError::Encode(e.to_string()))?;

    // If still too large, shrink further.
    let mut edge = MAX_IMAGE_EDGE;
    while out.len() > MAX_IMAGE_BYTES && edge > 256 {
        edge /= 2;
        let again = downscale(
            image::load_from_memory(bytes).map_err(|e| ImageError::Decode(e.to_string()))?,
            edge,
        );
        out.clear();
        again
            .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Jpeg)
            .map_err(|e| ImageError::Encode(e.to_string()))?;
    }

    let mime = match format {
        ImageFormat::Png => "image/png",
        _ => "image/jpeg",
    };
    Ok((out, mime.into()))
}

fn downscale(img: DynamicImage, max_edge: u32) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    if w <= max_edge && h <= max_edge {
        return img;
    }
    let scale = (max_edge as f32 / w.max(h) as f32).min(1.0);
    let nw = ((w as f32) * scale).round().max(1.0) as u32;
    let nh = ((h as f32) * scale).round().max(1.0) as u32;
    img.resize(nw, nh, FilterType::Triangle)
}

fn choose_format(mime_hint: Option<&str>) -> ImageFormat {
    match mime_hint {
        Some(m) if m.contains("png") => ImageFormat::Png,
        _ => ImageFormat::Jpeg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png() -> Vec<u8> {
        // 1x1 PNG
        let img = DynamicImage::new_rgb8(1, 1);
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), ImageFormat::Png)
            .unwrap();
        buf
    }

    #[test]
    fn resizes_without_error() {
        let (out, mime) = resize_for_provider(&tiny_png(), Some("image/png")).unwrap();
        assert!(!out.is_empty());
        assert!(mime.starts_with("image/"));
    }
}
