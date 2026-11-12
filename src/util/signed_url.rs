//! HMAC-signed, expiring URLs for the image server.
//!
//! Image widgets can't attach an `Authorization` header, so instead the
//! (authenticated) gRPC calls hand out URLs carrying `exp` + `sig` query
//! params, and the image server only checks the signature.
//!
//! `exp` is bucketed to a whole `BUCKET_SECS` boundary so a given resource
//! gets the same URL for a full bucket -- otherwise every gRPC call would mint
//! a new URL and defeat client-side image caches keyed on the URL. A URL is
//! valid for between one and two buckets after it's handed out.

use hmac::Mac;

use crate::interceptor::auth::SECRET_KEY;

const BUCKET_SECS: i64 = 24 * 60 * 60;

/// Domain-separates these signatures from the JWTs signed with the same key.
const PREFIX: &[u8] = b"rumgap-image-url\n";

fn mac(resource: &str, exp: i64) -> hmac::Hmac<sha2::Sha256> {
    let mut mac = SECRET_KEY.clone();
    mac.update(PREFIX);
    mac.update(exp.to_string().as_bytes());
    mac.update(b"\n");
    mac.update(resource.as_bytes());
    mac
}

/// `(exp, sig)` for `resource`, expiring at the end of the next bucket.
pub fn sign(resource: &str) -> (i64, String) {
    let now = chrono::Utc::now().timestamp();
    let exp = (now / BUCKET_SECS + 2) * BUCKET_SECS;
    let sig = hex::encode(mac(resource, exp).finalize().into_bytes());
    (exp, sig)
}

/// True if `sig` is a valid, unexpired signature of `resource`.
pub fn verify(resource: &str, exp: Option<i64>, sig: Option<&str>) -> bool {
    let (Some(exp), Some(sig)) = (exp, sig) else {
        return false;
    };
    if exp < chrono::Utc::now().timestamp() {
        return false;
    }
    let Ok(sig) = hex::decode(sig) else {
        return false;
    };
    mac(resource, exp).verify_slice(&sig).is_ok()
}

fn base_url() -> String {
    let base_url = std::env::var("IMAGE_BASE_URL").unwrap_or_else(|_| "http://localhost:8001".to_string());
    base_url.trim_end_matches('/').to_string()
}

/// `{IMAGE_BASE_URL}{path}?exp=...&sig=...`, where `path` is also the signed resource.
pub fn path_url(path: &str) -> String {
    let (exp, sig) = sign(path);
    format!("{}{path}?exp={exp}&sig={sig}", base_url())
}

/// Signed resource for a `/proxy` request -- covers both query params, so a
/// signature can't be replayed against a different target.
pub fn proxy_resource(url: &str, referer: &str) -> String {
    format!("/proxy\n{url}\n{referer}")
}

/// `{IMAGE_BASE_URL}/proxy?url=...&referer=...&exp=...&sig=...`
///
/// Rewrites a raw scraped cover URL into rumgap's own transient `/proxy`
/// cache (Phase A2) -- the client never sees or needs the raw source URL,
/// and the server does the Referer-spoofing itself rather than the client
/// (which matters beyond tidiness: a browser build can't set `Referer` on
/// `fetch`/XHR at all, it's a forbidden header).
pub fn proxy_url(url: &str, referer: &str) -> String {
    let (exp, sig) = sign(&proxy_resource(url, referer));
    let mut proxy = manga_parser::Url::parse(&format!("{}/proxy", base_url()))
        .expect("IMAGE_BASE_URL should be a valid base URL");
    proxy
        .query_pairs_mut()
        .append_pair("url", url)
        .append_pair("referer", referer)
        .append_pair("exp", &exp.to_string())
        .append_pair("sig", &sig);
    proxy.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        std::env::set_var("JWT_SECRET", "test");
        let (exp, sig) = sign("/covers/1");
        assert!(verify("/covers/1", Some(exp), Some(&sig)));
        assert!(!verify("/covers/2", Some(exp), Some(&sig)));
        assert!(!verify("/covers/1", Some(exp + 1), Some(&sig)));
        assert!(!verify("/covers/1", Some(exp), None));
        assert!(!verify("/covers/1", Some(0), Some(&hex::encode(mac("/covers/1", 0).finalize().into_bytes()))));
    }
}
