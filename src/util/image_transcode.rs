//! Downscale + recompress a chapter page image as a smaller JPEG, on demand,
//! parameterized entirely by the caller's `(Bound, quality)` -- there's no
//! separate "quality" vs "render-safe" mode server-side, just the same
//! operation at different presets the client chooses:
//! - Reader quality (user-selected): capped by width ([`Bound::Width`]) so
//!   long-strip pages keep a readable width instead of being shrunk until
//!   their height fits.
//! - Render-safe (Flutter Web only): some mobile GPUs (seen via CanvasKit's
//!   `texImage2D: width or height out of range`) reject a texture upload
//!   past a certain size, and long-strip webtoon pages routinely exceed it.
//!   Flutter's `cacheWidth`/`cacheHeight` decode-time resize hints don't
//!   help here -- CanvasKit decodes network images via the browser,
//!   ignoring them -- so the cap has to be applied to the bytes
//!   themselves, before they ever reach the client. Native platforms never
//!   hit this limit, so they ask for no transform at all.
//!
//! Pure functions plus key/etag derivation -- no I/O here. `image_server.rs`
//! owns reading the original from `IMAGE_STORE`, calling [`transcode_if_over`],
//! and caching the result; `chapter_images.rs`'s `refresh_chapter_images`
//! uses `ImageStore::delete_prefix` to invalidate every cached variant
//! (whatever `(Bound, quality)` pairs happen to exist) alongside the
//! original on refresh.

use image::imageops::FilterType;
use image::ImageReader;

/// Which side of the image a size cap applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bound {
    /// Cap the longest side (`?maxdim=`). For long strips this is the height,
    /// so it shrinks them to a sliver -- only meant for render-safety caps.
    Longest(u32),
    /// Cap the width only (`?maxwidth=`), height follows the aspect ratio.
    Width(u32),
}

impl Bound {
    fn limit(self) -> u32 {
        match self {
            Bound::Longest(limit) | Bound::Width(limit) => limit,
        }
    }

    /// The side of a `width x height` image this bound is measured against.
    fn measured(self, width: u32, height: u32) -> u32 {
        match self {
            Bound::Longest(_) => width.max(height),
            Bound::Width(_) => width,
        }
    }

    /// Whether a `width x height` image is larger than this bound allows.
    pub fn exceeded_by(self, width: u32, height: u32) -> bool {
        self.measured(width, height) > self.limit()
    }

    /// Short tag baked into cache keys and etags, e.g. `v1280` or `w1280`.
    fn tag(self) -> String {
        match self {
            Bound::Longest(limit) => format!("v{limit}"),
            Bound::Width(limit) => format!("w{limit}"),
        }
    }
}

/// Derive the `IMAGE_STORE` key for a transcoded variant of an original page
/// keyed by `storage_key` (e.g. `"42/3.jpg"` -> `"42/3.jpg.v1280q70.jpg"`).
/// Config is baked into the key so a different `(Bound, quality)` request
/// naturally gets its own cache lineage instead of colliding with another
/// caller's variant.
pub fn variant_key(storage_key: &str, bound: Bound, quality: u8) -> String {
    format!("{storage_key}.{}q{quality}.jpg", bound.tag())
}

/// Derive the `ETag` for a transcoded variant from the original's checksum.
/// Deterministic given `(checksum, Bound, quality)`, so it can be computed
/// -- and matched against `If-None-Match` -- without touching disk.
pub fn variant_etag(checksum: &str, bound: Bound, quality: u8) -> String {
    format!("{checksum}-{}q{quality}", bound.tag())
}

/// Decode `bytes`, downscale (only if larger than `bound`, preserving aspect
/// ratio) using Lanczos3, and re-encode as JPEG at `quality`. Returns `Err`
/// for anything undecodable (e.g. AVIF, decode support not enabled) so the
/// caller can fall back to serving the original bytes untouched.
fn transcode(bytes: &[u8], bound: Bound, quality: u8) -> Result<Vec<u8>, String> {
    let img = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;

    let (width, height) = (img.width(), img.height());

    let resized = if bound.exceeded_by(width, height) {
        let scale = bound.limit() as f64 / bound.measured(width, height) as f64;
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

/// Cheap header-only check (no pixel decode) of whether `bytes` is larger
/// than `bound`. `Err` for anything whose format/dimensions can't even be read.
pub fn exceeds_dimension(bytes: &[u8], bound: Bound) -> Result<bool, String> {
    let (width, height) = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_dimensions()
        .map_err(|e| e.to_string())?;
    Ok(bound.exceeded_by(width, height))
}

/// Returns `Ok(None)` -- "serve the original bytes untouched" -- when the
/// source is already within `bound`, instead of always taking a lossy
/// JPEG round-trip regardless of whether shrinking was actually needed.
pub fn transcode_if_over(bytes: &[u8], bound: Bound, quality: u8) -> Result<Option<Vec<u8>>, String> {
    if !exceeds_dimension(bytes, bound)? {
        return Ok(None);
    }
    transcode(bytes, bound, quality).map(Some)
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
    fn transcode_if_over_leaves_small_images_untouched() {
        assert!(transcode_if_over(&png_fixture(640, 480), Bound::Longest(1280), 70)
            .unwrap()
            .is_none());
    }

    #[test]
    fn transcode_if_over_shrinks_oversized_images() {
        let out = transcode_if_over(&png_fixture(3840, 2160), Bound::Longest(1280), 70)
            .unwrap()
            .unwrap();
        let decoded = image::load_from_memory(&out).unwrap();
        assert_eq!(decoded.width(), 1280);
        assert_eq!(decoded.height(), 720);
    }

    #[test]
    fn caps_long_strip_pages() {
        let out = transcode_if_over(&png_fixture(800, 20000), Bound::Longest(4096), 92)
            .unwrap()
            .unwrap();
        let decoded = image::load_from_memory(&out).unwrap();
        assert_eq!(decoded.height(), 4096);
        assert!(decoded.width() < 800);
    }

    #[test]
    fn rejects_undecodable_bytes() {
        assert!(transcode_if_over(b"not an image", Bound::Longest(1280), 70).is_err());
    }

    #[test]
    fn width_bound_keeps_long_strips_readable() {
        let out = transcode_if_over(&png_fixture(1600, 20000), Bound::Width(800), 70)
            .unwrap()
            .unwrap();
        let decoded = image::load_from_memory(&out).unwrap();
        assert_eq!(decoded.width(), 800);
        assert_eq!(decoded.height(), 10000);
    }

    #[test]
    fn width_bound_ignores_height() {
        assert!(transcode_if_over(&png_fixture(720, 20000), Bound::Width(1080), 70)
            .unwrap()
            .is_none());
    }

    #[test]
    fn key_and_etag_bake_in_params() {
        assert_eq!(
            variant_key("42/3.jpg", Bound::Longest(1280), 70),
            "42/3.jpg.v1280q70.jpg"
        );
        assert_eq!(variant_etag("abc123", Bound::Longest(1280), 70), "abc123-v1280q70");
        assert_eq!(variant_key("42/3.jpg", Bound::Width(1080), 75), "42/3.jpg.w1080q75.jpg");
        assert_eq!(variant_etag("abc123", Bound::Width(1080), 75), "abc123-w1080q75");
    }
}
