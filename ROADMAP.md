# Roadmap

Covers both rumgap (this repo) and the [wuxia](https://github.com/hubble459/wuxia) client.

## To verify
- Cover images (`get_cover` in `image_server.rs`) still go through `serve_from_store` with no
  render-safe cap, while pages go through `serve_variant` with `maxdim`. Low risk (covers
  aren't long-strip format), but inconsistent — worth applying the same treatment if a source
  ever serves an oversized cover.

## Next up
- **Right-to-left paged reading** (wuxia). The paged reader is a plain `PageView.builder` in
  `manga_chapter_screen.dart`; needs `reverse: true` behind a reading-mode option.
- **Manual chapter link/unlink UI** (wuxia). `LinkChapter`/`UnlinkChapter` exist server-side
  but have no UI. Would let you fix a mismatched canonical-chapter pairing by hand. The
  `FAILED_PRECONDITION` is currently swallowed in `manga_chapter_screen.dart`, next to a TODO.
- **Show sources that are down** (wuxia). Grey out / mark manga whose website is down, using
  the data `Scraper.Status` already exposes (see the admin scraper status screen).
- **Keep the bottom bar's chapter window in memory** (wuxia). `manga_bottom_bar.dart` refetches
  it on every rebuild (TODO from Nov 2023).
- **Source-per-scanlation-group** (rumgap + wuxia). Bigger architectural idea: model
  MangaDex's multiple scanlation groups as switchable "sources" (reusing the existing
  switch-source UI) instead of relying on canonical_chapter linking. Requires pulling group
  info out of the scraper and reworking the source/switcher model.

## Wishlist
- Social: friends, profile page with recent activity (like MAL), custom avatar, ratings,
  comments
- MAL integration
- Daily activity chart
- Priority calculator for the background manga updater
- Tests (rumgap + wuxia)
- Periodic backups
- API documentation and code comments (rumgap)

## Not planned
- Making "Add Manga" / Search admin-only — only trusted people get an account.
- Optional email / forgot-password flow — admins create accounts and reset passwords.

## Done
- Merge duplicate manga / move a source between mangas (`MergeManga`/`MoveSource`, admin menu
  in wuxia)
- Admin user management, closed registration, change password
- Downloads and offline reading (native, and web via the File System Access API or OPFS),
  CBZ export
- Paged image view (top-down and paged)
- gRPC over TLS (dedicated port 5909)
- Notifications for new chapters (background updater + FCM)
- Downsampled reader images (server-side render-safe transcode, `maxdim`/`quality`)
- Add from URL, open manga/chapter in browser, search alternatives
- Progress per chapter
- Advise choosing alternatives when a source is dead
- Update app from within settings
