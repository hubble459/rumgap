# Roadmap

Covers both rumgap (this repo) and the [wuxia](https://github.com/hubble459/wuxia) client.

## Next up
- **Source-per-scanlation-group** (rumgap + wuxia). Bigger architectural idea: model
  MangaDex's multiple scanlation groups as switchable "sources" (reusing the existing
  switch-source UI) instead of relying on canonical_chapter linking. Requires pulling group
  info out of the scraper and reworking the source/switcher model. Also covers crediting the
  scanlation groups, which MangaDex's API usage policy requires.
- **Try For You on a real device**: swipe feel, chapter-1 preview, filters, skip, 18+ chip.

## Wishlist
- Social: custom avatar, ratings
- MAL / AniList integration
- Priority calculator for the background updater. Today it picks the manga with the most
  readers, then the longest-unrefreshed; a real one would weigh the expected next release,
  ongoing/finished and how often a manga updates.
- Cover cache-busting: put a version in cover urls so a broken cover cached by clients
  (`Cache-Control: immutable`) can't outlive a re-download
- Source languages, once a non-English site is added: a `language` per scraper config,
  stored on each source, a `language:` filter, and a "my languages" setting that hides
  sources and manga in languages you can't read
- Tests (rumgap + wuxia)
- Periodic backups
- API documentation and code comments (rumgap)

## Not planned
- Making "Add Manga" / Search admin-only — only trusted people get an account.
- Optional email / forgot-password flow — admins create accounts and reset passwords.

## Done
- For You swipe deck: content-based ranking of manga you don't read, not-interested swipes
  (hidden 90 days, count against similar manga), skip, undo, chapter-1 preview, filter in the
  search syntax
- 18+ flag per manga (adult-only sites via manga_parser `nsfw: true`, MangaDex adult ratings,
  mod/admin toggle), per-user "Show 18+ manga" setting
- Search merged with Latest: library and online results together, every host searched, one
  slow host can't fail a search; preferred sources moved to settings
- Online search results saved to the catalog in the background, added to an existing manga
  when their title matches exactly
- Genre normalization (one spelling per genre, aliases such as Sci-Fi and Reincarnation),
  format and status tags (Long Strip, Full Color, Completed, ...) dropped from genres
- One url and hostname for MangaDex; honest User-Agent for MangaDex image downloads
- Chapters a site keeps behind a login (early access) reported as such
- Social: friends, follow by username, profiles with read history, private profiles,
  threaded manga comments
- Detailed reading statistics, daily activity chart
- Admin storage page (disk, database, per site and manga)
- Manga details per source, each user sees their selected source
- Reading and browsing chapters of manga outside the reading list
- Image quality setting (width-based cap) instead of data saver
- Login sessions that can be revoked
- Mods and admins can delete a manga
- App-wide download queue that survives restarts
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
