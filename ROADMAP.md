# Roadmap

## To verify
- Confirm the "pages randomly render black" bug is actually gone after real, extended
  reading on-device, now that the fix has shipped. Root cause turned out to be unrelated to
  `CachedNetworkImage`/disk-cache corruption (already ruled out by the earlier
  `CachedNetworkImage` -> `Image.network` swap, which didn't fix it): Flutter Web's CanvasKit
  renderer ignores `cacheWidth`/`cacheHeight` decode hints for network images (decoded via
  the browser, not Flutter's codec path), so any long-strip webtoon page taller than the
  device's `GL_MAX_TEXTURE_SIZE` crashed the WebGL texture upload and rendered solid black
  forever. Fixed server-side instead (rumgap `image_server.rs`/`image_transcode.rs`): every
  served page is capped at 4096px on its longest side (JPEG q92, only when actually over the
  cap — everything else is served byte-for-byte untouched), independent of the client's
  data-saver preference.
- Cover images (`get_cover` in `image_server.rs`) still go through the old
  `serve_from_store` with no render-safe cap at all. Low risk (covers aren't long-strip
  format), but inconsistent with how pages are now handled — worth applying the same
  render-safe treatment there too if a source ever serves an oversized cover.

## Features
- **Merge duplicate manga / move a source between mangas** (rumgap). `FindOrCreate` only
  dedupes by exact URL, so adding the same title from a different source creates a second
  manga entry. No safe way to merge today — `RemoveSource` deletes the manga_source and
  cascades away its chapters. Needs a real `MergeManga`/`MoveSource` admin RPC (or at
  minimum documented raw-SQL steps: re-parent `manga_source.manga_id`, null + relink
  `canonical_chapter_id`).
- Related: consider whether "Add Manga" / the general Search page should be admin-only,
  given how easy it is to create duplicate mangas today.
- **Manual chapter link/unlink UI** (wuxia). `LinkChapter`/`UnlinkChapter` exist server-side
  but have no UI. Would let you fix a mismatched canonical-chapter pairing by hand. TODO
  comments already left in `manga_chapters_screen.dart`/`manga_chapter_screen.dart` where
  the `FAILED_PRECONDITION` is currently just silently swallowed.
- **Source-per-scanlation-group** (rumgap + wuxia). Bigger architectural idea: model
  MangaDex's multiple scanlation groups as switchable "sources" (reusing the existing
  switch-source UI) instead of relying on canonical_chapter linking. Nicer UX than
  "duplicates just share progress," but requires pulling group info out of the scraper and
  reworking the source/switcher model.
- ~~Downsample reader images to display size~~ Done — capped both client-side (native only;
  `ResizeImagePolicy.fit` in `manga_chapter_screen.dart`, ignored by CanvasKit on web) and,
  more importantly, server-side (render-safe transcode, all platforms) as part of the
  black-image fix above.
- **Progressive image loading (low priority)** (wuxia). On the manga website, a slow-loading
  image visibly buffers in top-to-bottom so you can start reading before it fully loads -
  browsers do this natively for progressive JPEGs. Flutter's image widgets don't replicate
  this out of the box (`loadingBuilder` only reports download progress, not partial-pixel
  rendering); would need a custom incremental decoder. Worth doing, not urgent. Unrelated to
  the black-image fix (doesn't affect and isn't affected by it) — except that the rare
  render-safe-resized long-strip pages are re-encoded as baseline JPEG (`image` crate's
  encoder has no progressive mode), so those specific pages wouldn't benefit even if this
  got built; would need a different encoder for that subset.
