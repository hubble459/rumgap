# Roadmap

Covers both rumgap (this repo) and the [wuxia](https://github.com/hubble459/wuxia) client.

## Next up
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
- Paged image view (top-down, paged, right-to-left)
- Linking/unlinking chapters across sources by hand
- Marking sources whose website is down
- `maxdim`/`quality` cap on cover images, same as pages
- gRPC over TLS (dedicated port 5909)
- Notifications for new chapters (background updater + FCM)
- Downsampled reader images (server-side render-safe transcode, `maxdim`/`quality`)
- Add from URL, open manga/chapter in browser, search alternatives
- Progress per chapter
- Advise choosing alternatives when a source is dead
- Update app from within settings
