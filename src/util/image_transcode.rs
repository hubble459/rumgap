//! Downscale + recompress a chapter page image as a smaller JPEG. Two
//! variants, both backed by the same [`transcode`]:
//! - Data-saver (`?ds=true`): user-selected, aggressively downscaled to
//!   save bandwidth.
//! - Render-safe (the default, always on): some mobile GPUs (seen via
//!   Flutter Web/CanvasKit's `texImage2D: width or height out of range`)
//!   reject a texture upload past a certain size, and long-strip webtoon
//!   pages routinely exceed it. Flutter's `cacheWidth`/`cacheHeight`
//!   decode-time resize hints don't help here -- CanvasKit decodes network
//!   images via the browser, ignoring them -- so the cap has to be applied
//!   to the bytes themselves, before they ever reach the client.
//!
//! Pure function plus config accessors and key/etag derivation -- no I/O
//! here. `image_server.rs` owns reading the original from `IMAGE_STORE`,
//! calling [`transcode`], and caching the result; `chapter_images.rs`'s
//! `refresh_chapter_images` uses [`data_saver_key`]/[`render_safe_key`] to
//! invalidate stale cached variants alongside the original on refresh.

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

/// Longest-side cap in pixels for the always-on render-safe variant --
/// comfortably under GPU texture-size limits seen in the wild on mobile.
/// Far above the data-saver cap, since this exists to keep the client's
/// renderer from choking, not to save bandwidth.
pub fn render_safe_max_dimension() -> u32 {
    std::env::var("IMAGE_RENDER_MAX_DIMENSION")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(4096)
}

/// JPEG quality for the render-safe variant. High enough that recompressing
/// every page (even ones that weren't oversized to begin with) is visually
/// lossless -- this isn't a bandwidth optimization, so it doesn't need
/// data-saver's aggressive quality tradeoff.
pub const RENDER_SAFE_QUALITY: u8 = 92;

/// Same idea as [`data_saver_key`], for the render-safe variant.
pub fn render_safe_key(storage_key: &str) -> String {
    format!("{storage_key}.rs{}q{}.jpg", render_safe_max_dimension(), RENDER_SAFE_QUALITY)
}

/// Same idea as [`data_saver_etag`], for the render-safe variant.
pub fn render_safe_etag(checksum: &str) -> String {
    format!("{checksum}-rs{}q{}", render_safe_max_dimension(), RENDER_SAFE_QUALITY)
}

/// Decode `bytes`, downscale (only if larger than `max_dim` on its longest
/// side, preserving aspect ratio) using Lanczos3, and re-encode as JPEG at
/// `quality`. Returns `Err` for anything undecodable (e.g. AVIF, decode
/// support not enabled) so the caller can fall back to serving the original
/// bytes untouched.
///
/// Always re-encodes, even when no resize is needed -- data-saver wants
/// that (a smaller/lower-quality file regardless of source dimensions). For
/// the render-safe path, which should leave already-safe images completely
/// untouched, use [`transcode_if_over`] instead.
pub fn transcode(bytes: &[u8], max_dim: u32, quality: u8) -> Result<Vec<u8>, String> {
    let img = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;

    let (width, height) = (img.width(), img.height());
    let longest = width.max(height);

    let resized = if longest > max_dim {
        let scale = max_dim as f64 / longest as f64;
        let new_width = ((width as f64 * scale).round() as u32).max(1);
        let new_height = ((height as f64 * scale).round() as u32).max(1);
        img.resize(new_width, new_height, FilterType::Lanczos3)
    } else {
        img
    };

    let mut out = Vec::new();
    // JPEG has no alpha channel -- flatten any transparency to opaque.
    let rgb = resized.to_rgb8();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    rgb.write_with_encoder(encoder).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Cheap header-only check (no pixel decode) of whether `bytes`' longest
/// side exceeds `max_dim`. `Err` for anything whose format/dimensions can't
/// even be read.
pub fn exceeds_dimension(bytes: &[u8], max_dim: u32) -> Result<bool, String> {
    let (width, height) = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_dimensions()
        .map_err(|e| e.to_string())?;
    Ok(width.max(height) > max_dim)
}

/// Like [`transcode`], but returns `Ok(None)` -- "serve the original bytes
/// untouched" -- when the source is already within `max_dim`, instead of
/// always taking a lossy JPEG round-trip regardless of whether shrinking
/// was actually needed.
pub fn transcode_if_over(bytes: &[u8], max_dim: u32, quality: u8) -> Result<Option<Vec<u8>>, String> {
    if !exceeds_dimension(bytes, max_dim)? {
        return Ok(None);
    }
    transcode(bytes, max_dim, quality).map(Some)
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

    #[test]
    fn downscales_when_over_the_cap() {
        let out = transcode(&png_fixture(3840, 2160), 1280, 70).unwrap();
        let decoded = image::load_from_memory(&out).unwrap();
        assert_eq!(decoded.width(), 1280);
        assert_eq!(decoded.height(), 720);
    }

    #[test]
    fn never_upscales_when_under_the_cap() {
        let out = transcode(&png_fixture(640, 480), 1280, 70).unwrap();
        let decoded = image::load_from_memory(&out).unwrap();
        assert_eq!(decoded.width(), 640);
        assert_eq!(decoded.height(), 480);
    }

    #[test]
    fn rejects_undecodable_bytes() {
        assert!(transcode(b"not an image", 1280, 70).is_err());
    }

    #[test]
    fn caps_long_strip_pages_at_the_render_safe_dimension() {
        let out = transcode(&png_fixture(800, 20000), render_safe_max_dimension(), RENDER_SAFE_QUALITY).unwrap();
        let decoded = image::load_from_memory(&out).unwrap();
        assert_eq!(decoded.height(), render_safe_max_dimension());
        assert!(decoded.width() < 800);
    }

    #[test]
    fn transcode_if_over_leaves_small_images_untouched() {
        assert!(transcode_if_over(&png_fixture(640, 480), 1280, 70).unwrap().is_none());
    }

    #[test]
    fn transcode_if_over_shrinks_oversized_images() {
        let out = transcode_if_over(&png_fixture(800, 20000), 4096, 92).unwrap().unwrap();
        let decoded = image::load_from_memory(&out).unwrap();
        assert_eq!(decoded.height(), 4096);
    }

    #[test]
    fn key_and_etag_bake_in_config() {
        assert_eq!(data_saver_key("42/3.jpg"), "42/3.jpg.ds1280q70.jpg");
        assert_eq!(data_saver_etag("abc123"), "abc123-ds1280q70");
        assert_eq!(render_safe_key("42/3.jpg"), "42/3.jpg.rs4096q92.jpg");
        assert_eq!(render_safe_etag("abc123"), "abc123-rs4096q92");
    }
}
