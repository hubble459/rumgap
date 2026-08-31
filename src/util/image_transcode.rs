//! Data-saver transcode: downscale + recompress a chapter page image as a
//! smaller JPEG, for the `?ds=true` query param on the image server.
//!
//! Pure function plus config accessors and key/etag derivation -- no I/O
//! here. `image_server.rs` owns reading the original from `IMAGE_STORE`,
//! calling [`transcode`], and caching the result; `chapter_images.rs`'s
//! `refresh_chapter_images` uses [`data_saver_key`] to invalidate a stale
//! cached variant alongside the original on refresh.

use image::imageops::FilterType;
use image::ImageReader;

/// Longest-side cap in pixels for the data-saver variant. Downscale only,
/// never upscale.
pub fn max_dimension() -> u32 {
    std::env::var("DATA_SAVER_MAX_DIMENSION")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(1280)
}

/// JPEG quality (1-100) for the re-encoded data-saver variant.
pub fn quality() -> u8 {
    std::env::var("DATA_SAVER_JPEG_QUALITY")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v| (1..=100).contains(v))
        .unwrap_or(70)
}

/// Derive the `IMAGE_STORE` key for the data-saver variant of an original
/// page keyed by `storage_key` (e.g. `"42/3.jpg"` -> `"42/3.jpg.ds1280q70.jpg"`).
/// Config is baked into the key so changing `DATA_SAVER_MAX_DIMENSION`/
/// `DATA_SAVER_JPEG_QUALITY` naturally starts a fresh cache lineage instead
/// of silently serving stale-quality bytes under an unchanged key.
pub fn data_saver_key(storage_key: &str) -> String {
    format!("{storage_key}.ds{}q{}.jpg", max_dimension(), quality())
}

/// Derive the `ETag` for a data-saver variant from the original's checksum.
/// Deterministic given `(checksum, max_dimension, quality)`, so it can be
/// computed -- and matched against `If-None-Match` -- without touching disk.
pub fn data_saver_etag(checksum: &str) -> String {
    format!("{checksum}-ds{}q{}", max_dimension(), quality())
}

/// Decode `bytes`, downscale (only if larger than `max_dimension()` on its
/// longest side, preserving aspect ratio) using Lanczos3, and re-encode as
/// JPEG at `quality()`. Returns `Err` for anything undecodable (e.g. AVIF,
/// decode support not enabled) so the caller can fall back to serving the
/// original bytes untouched.
pub fn transcode(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let img = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;

    let (width, height) = (img.width(), img.height());
    let longest = width.max(height);
    let cap = max_dimension();

    let resized = if longest > cap {
        let scale = cap as f64 / longest as f64;
        let new_width = ((width as f64 * scale).round() as u32).max(1);
        let new_height = ((height as f64 * scale).round() as u32).max(1);
        img.resize(new_width, new_height, FilterType::Lanczos3)
    } else {
        img
    };

    let mut out = Vec::new();
    // JPEG has no alpha channel -- flatten any transparency to opaque.
    let rgb = resized.to_rgb8();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality());
    rgb.write_with_encoder(encoder).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_fixture(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        let mut bytes = Vec::new();
        img.write_with_encoder(image::codecs::png::PngEncoder::new(&mut bytes))
            .unwrap();
        bytes
    }

    // These rely on the *default* max_dimension (1280) / quality (70) rather
    // than overriding the env vars, since cargo runs tests in parallel
    // threads within the same process and a set_var/remove_var pair here
    // would race with the other tests in this module.

    #[test]
    fn downscales_when_over_the_cap() {
        let out = transcode(&png_fixture(3840, 2160)).unwrap();
        let decoded = image::load_from_memory(&out).unwrap();
        assert_eq!(decoded.width(), 1280);
        assert_eq!(decoded.height(), 720);
    }

    #[test]
    fn never_upscales_when_under_the_cap() {
        let out = transcode(&png_fixture(640, 480)).unwrap();
        let decoded = image::load_from_memory(&out).unwrap();
        assert_eq!(decoded.width(), 640);
        assert_eq!(decoded.height(), 480);
    }

    #[test]
    fn rejects_undecodable_bytes() {
        assert!(transcode(b"not an image").is_err());
    }

    #[test]
    fn key_and_etag_bake_in_config() {
        assert_eq!(data_saver_key("42/3.jpg"), "42/3.jpg.ds1280q70.jpg");
        assert_eq!(data_saver_etag("abc123"), "abc123-ds1280q70");
    }
}
