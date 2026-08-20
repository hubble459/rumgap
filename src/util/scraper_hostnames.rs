//! Allowlist of hostnames the `/proxy` route (Phase A2) is willing to fetch
//! from -- built from the same `configs/*.yaml` files `GenericScraper`
//! already loads, plus MangaDex's known hostnames. Exists purely to keep
//! `/proxy` (unauthenticated, potentially internet-facing) from becoming a
//! generic fetch-arbitrary-URL SSRF proxy: without this, anyone who finds
//! the endpoint -- not just wuxia -- could use rumgap to reach internal
//! network addresses, cloud metadata endpoints, etc.
//!
//! Known limitation: a site that's only accepted via DOM-fingerprint
//! selectors (no explicit `hostnames` entry in its config, e.g. some of
//! madara.yaml's generic acceptance) won't pass this allowlist unless its
//! hostname is explicitly added to that config -- a one-line YAML edit if a
//! specific site's covers ever need proxying and its host isn't already
//! listed for search/accept purposes.

use std::collections::HashSet;
use std::path::Path;

use config::{builder::DefaultState, ConfigBuilder, File};
use manga_parser::config::MangaScraperConfig;

/// MangaDex isn't config-driven (it's a hardcoded scraper backed by the
/// official API), so its hostnames aren't in `configs/*.yaml` -- listed
/// here instead. Covers are served from MangaDex's uploads CDN.
const MANGADEX_HOSTNAMES: &[&str] = &["mangadex.org", "api.mangadex.org", "uploads.mangadex.org"];

lazy_static! {
    static ref ALLOWED_HOSTNAMES: HashSet<String> = load_allowed_hostnames();
}

fn load_allowed_hostnames() -> HashSet<String> {
    let mut hostnames: HashSet<String> = MANGADEX_HOSTNAMES.iter().map(|s| s.to_string()).collect();

    let configs_dir = Path::new("configs");
    let Ok(entries) = std::fs::read_dir(configs_dir) else {
        warn!("Could not read configs/ directory for hostname allowlist - only MangaDex hosts allowed");
        return hostnames;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let is_yaml = path.extension().is_some_and(|ext| ext == "yaml" || ext == "yml");
        if !is_yaml {
            continue;
        }

        let parsed = ConfigBuilder::<DefaultState>::default()
            .add_source(File::from(path.clone()))
            .build()
            .and_then(|c| c.try_deserialize::<MangaScraperConfig>());

        match parsed {
            Ok(scraper_config) => {
                hostnames.extend(scraper_config.accept.hostnames.iter().cloned());
                for search in &scraper_config.search {
                    hostnames.extend(search.hostnames.iter().cloned());
                }
            }
            Err(e) => warn!("Failed to parse {} for hostname allowlist: {}", path.display(), e),
        }
    }

    hostnames
}

/// Whether `hostname` is a recognized manga-scraping target. Case-sensitive
/// exact match against the loaded allowlist (hostnames in configs/*.yaml are
/// already lowercase) - callers should lowercase the incoming host first.
pub fn is_allowed_hostname(hostname: &str) -> bool {
    ALLOWED_HOSTNAMES.contains(hostname)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mangadex_hostnames_always_allowed() {
        assert!(is_allowed_hostname("mangadex.org"));
        assert!(is_allowed_hostname("uploads.mangadex.org"));
    }

    #[test]
    fn unrelated_hostnames_rejected() {
        assert!(!is_allowed_hostname("169.254.169.254"));
        assert!(!is_allowed_hostname("localhost"));
        assert!(!is_allowed_hostname("example.com"));
    }
}
