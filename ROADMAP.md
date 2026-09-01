# Roadmap

## To verify
- Confirm the `Image.network` swap in the chapter reader (wuxia) actually resolves the
  "pages randomly render black" bug under real, extended reading — not just a quick local
  test. Suspected cause: `cached_network_image`'s disk cache getting corrupted by a fetch
  cancelled mid-download when a page scrolls out of view.
- The same class of bug could still affect the other three spots still using
  `CachedNetworkImage` (manga cover thumbnails: `manga_item.dart`, `search_manga_item.dart`,
  `manga_screen.dart`) — lower risk since those are single static images, not rapidly
  scrolled/disposed reader pages, but worth watching.

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
- **Downsample reader images to display size** (wuxia). `Image.network` decodes at full
  native resolution; specify `cacheWidth`/`cacheHeight` (based on actual rendered width) to
  cut peak memory use — the Flutter equivalent of what Glide did automatically on the old
  Android app.
