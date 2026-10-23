//! A single, hardened HTTP client shared by every source.
//!
//! * rustls only (no OpenSSL, no system TLS quirks);
//! * HTTPS enforced: plain `http://` URLs are refused before any request;
//! * short connect/overall timeouts so a stalled API can't freeze the UI;
//! * response bodies capped — we only ever read small JSON documents;
//! * no cookies, explicit User-Agent;
//! * redirects checked hop by hop, before they are followed: an API request
//!   stays on the host it was sent to, a download only ever touches the
//!   hosts in `download::ALLOWED_HOSTS`, and an addon icon those in
//!   `icons::ALLOWED_HOSTS` (see [`redirect_verdict`]).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use reqwest::redirect::Policy;
use reqwest::{Client, Url};
use serde::de::DeserializeOwned;

/// Most redirects followed for one request. GitHub's release assets need one
/// (`github.com` → `release-assets.githubusercontent.com`).
const MAX_REDIRECTS: usize = 3;

/// Maximum JSON body we are willing to buffer (1 MiB is far above any API answer).
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

const USER_AGENT: &str = concat!(
    "tocsin/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/fakejack/tocsin)"
);

/// Errors from [`Http`].
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    /// Not an `https://` URL.
    #[error("refusing non-HTTPS url: {0}")]
    InsecureUrl(String),
    /// Malformed URL.
    #[error("invalid url: {0}")]
    InvalidUrl(#[from] url::ParseError),
    /// Transport or status error.
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    /// Non-2xx response.
    #[error("{host} answered HTTP {status}")]
    Status {
        /// Host that answered.
        host: String,
        /// Status code.
        status: u16,
    },
    /// Body larger than the cap ([`MAX_BODY_BYTES`] unless the caller set one).
    #[error("the server's answer is larger than expected")]
    BodyTooLarge,
    /// A download ended up on a host it may not come from.
    #[error("refusing a download redirected to {0}")]
    UntrustedRedirect(String),
    /// The background decode did not finish.
    #[error("decoding the answer failed: {0}")]
    Task(String),
    /// JSON decode failure.
    #[error("invalid JSON from {url}: {source}")]
    Json {
        /// Request URL.
        url: String,
        /// Decode error.
        #[source]
        source: serde_json::Error,
    },
}

/// Thin wrapper around `reqwest::Client` with the policy baked in.
///
/// Carries a small response cache keyed by URL so that one check run asks
/// each endpoint once: the eleven BigWigs folders all resolve through the
/// same GitHub release list. [`Http::clear_cache`] starts a fresh run.
#[derive(Debug, Clone)]
pub struct Http {
    /// API requests: redirects only within the original host.
    client: Client,
    /// Release zips: redirects only to trusted download hosts, and a longer
    /// overall timeout, since a 20 MiB zip on a slow line takes a while.
    downloads: Client,
    /// Addon icons: redirects only to the icon hosts, and a short timeout,
    /// since a list icon that is slow to come is not worth waiting for.
    images: Client,
    cache: Arc<Mutex<HashMap<String, Bytes>>>,
    /// One async lock per URL so concurrent callers for the same endpoint
    /// wait for the first fetch instead of all hitting the network.
    inflight: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

impl Http {
    /// Build the clients. Panics only if TLS initialisation fails, which
    /// would make the whole program useless anyway.
    pub fn new() -> Self {
        let api = Policy::custom(|attempt| {
            let origin = attempt
                .previous()
                .first()
                .and_then(|u| u.host_str())
                .map(str::to_string);
            let verdict = redirect_verdict(attempt.previous(), attempt.url(), |host| {
                origin.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(host))
            });
            match verdict {
                Ok(()) => attempt.follow(),
                Err(why) => attempt.error(why),
            }
        });
        let downloads = Policy::custom(|attempt| {
            match redirect_verdict(
                attempt.previous(),
                attempt.url(),
                crate::download::is_trusted_host,
            ) {
                Ok(()) => attempt.follow(),
                Err(why) => attempt.error(why),
            }
        });
        let images = Policy::custom(|attempt| {
            match redirect_verdict(attempt.previous(), attempt.url(), crate::icons::is_allowed_host) {
                Ok(()) => attempt.follow(),
                Err(why) => attempt.error(why),
            }
        });
        Self {
            client: build(api, Duration::from_secs(20)),
            downloads: build(downloads, Duration::from_secs(300)),
            images: build(images, Duration::from_secs(10)),
            cache: Arc::new(Mutex::new(HashMap::new())),
            inflight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Forget cached responses (call at the start of a check run).
    pub fn clear_cache(&self) {
        if let Ok(mut c) = self.cache.lock() {
            c.clear();
        }
        if let Ok(mut i) = self.inflight.lock() {
            i.clear();
        }
    }

    fn cached(&self, key: &str) -> Option<Bytes> {
        self.cache.lock().ok().and_then(|c| c.get(key).cloned())
    }

    fn lock_for(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut map = match self.inflight.lock() {
            Ok(m) => m,
            Err(poisoned) => poisoned.into_inner(),
        };
        Arc::clone(map.entry(key.to_string()).or_default())
    }

    /// GET `url` for a file download: HTTPS enforced, status checked, body
    /// left to the caller to stream (see `download::fetch`). Never cached.
    pub async fn get_bytes(&self, url: &str) -> Result<reqwest::Response, HttpError> {
        let parsed = parse_https(url)?;
        let host = parsed.host_str().unwrap_or("server").to_string();
        let resp = self.downloads.get(parsed).send().await?;
        // The redirect policy already vetted every hop; checking where the
        // body actually comes from costs nothing and survives a policy change.
        let landed = resp.url().host_str().unwrap_or_default();
        if resp.url().scheme() != "https" || !crate::download::is_trusted_host(landed) {
            return Err(HttpError::UntrustedRedirect(landed.to_string()));
        }
        let status = resp.status();
        if !status.is_success() {
            return Err(HttpError::Status {
                host,
                status: status.as_u16(),
            });
        }
        Ok(resp)
    }

    /// GET `url` with optional extra headers and decode the JSON body.
    /// Cached until the next [`Http::clear_cache`]: meant for update checks,
    /// whose URLs are a bounded set per run.
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<T, HttpError> {
        let parsed = parse_https(url)?;
        // Cache hits never touch the network. The key includes the header
        // names so an authenticated request is not served an anonymous answer.
        let cache_key = {
            let mut k = url.to_string();
            for (name, _) in headers {
                k.push('|');
                k.push_str(name);
            }
            k
        };
        let decode = |bytes: &Bytes| {
            serde_json::from_slice(bytes).map_err(|source| HttpError::Json {
                url: url.to_string(),
                source,
            })
        };
        if let Some(bytes) = self.cached(&cache_key) {
            return decode(&bytes);
        }
        let gate = self.lock_for(&cache_key);
        let _guard = gate.lock().await;
        if let Some(bytes) = self.cached(&cache_key) {
            return decode(&bytes);
        }
        let bytes = self.fetch_capped(parsed, headers, MAX_BODY_BYTES).await?;
        if let Ok(mut c) = self.cache.lock() {
            c.insert(cache_key, bytes.clone());
        }
        decode(&bytes)
    }

    /// GET `url` and decode the JSON body, bypassing the response cache.
    /// Same HTTPS check, [`MAX_BODY_BYTES`] cap and redirect policy as
    /// [`Http::get_json`].
    ///
    /// For searches: every query is a new URL, so caching them would grow the
    /// cache with each search until the next check run, and a repeated search
    /// is rare enough not to be worth the memory.
    pub async fn get_json_uncached<T: DeserializeOwned>(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<T, HttpError> {
        let parsed = parse_https(url)?;
        let bytes = self.fetch_capped(parsed, headers, MAX_BODY_BYTES).await?;
        serde_json::from_slice(&bytes).map_err(|source| HttpError::Json {
            url: url.to_string(),
            source,
        })
    }

    /// POST `body` as JSON and decode the JSON answer, uncached. Same HTTPS
    /// check, [`MAX_BODY_BYTES`] cap and same-host redirect policy as
    /// [`Http::get_json`]. For documented batch lookups only (CurseForge's
    /// `POST /v1/mods`): one request instead of one per id.
    pub async fn post_json_uncached<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &B,
    ) -> Result<T, HttpError> {
        let parsed = parse_https(url)?;
        let host = parsed.host_str().unwrap_or("server").to_string();
        let payload = serde_json::to_vec(body).map_err(|source| HttpError::Json {
            url: url.to_string(),
            source,
        })?;
        let mut req = self
            .client
            .post(parsed)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .body(payload);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let bytes = read_capped(req.send().await?, host, MAX_BODY_BYTES).await?;
        serde_json::from_slice(&bytes).map_err(|source| HttpError::Json {
            url: url.to_string(),
            source,
        })
    }

    /// GET a large JSON document (a provider's whole catalog) and decode it
    /// off the async workers. `cap` is the caller's own limit, sized to the
    /// document with headroom; never cached here, since the caller keeps the
    /// decoded form and a copy of the raw bytes would only double the memory.
    pub async fn get_json_large<T: DeserializeOwned + Send + 'static>(
        &self,
        url: &str,
        cap: usize,
    ) -> Result<T, HttpError> {
        let parsed = parse_https(url)?;
        let bytes = self.fetch_capped(parsed, &[], cap).await?;
        let url = url.to_string();
        // Megabytes of JSON take tens of milliseconds to decode: not on a
        // worker that other requests are waiting on.
        tokio::task::spawn_blocking(move || {
            serde_json::from_slice(&bytes).map_err(|source| HttpError::Json { url, source })
        })
        .await
        .map_err(|e| HttpError::Task(e.to_string()))?
    }

    /// Send an API GET and read the body, at most `cap` bytes of it
    /// (see [`read_capped`]).
    async fn fetch_capped(&self, url: Url, headers: &[(&str, &str)], cap: usize) -> Result<Bytes, HttpError> {
        let host = url.host_str().unwrap_or("server").to_string();
        let mut req = self.client.get(url).header("Accept", "application/json");
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = req.send().await?;
        read_capped(resp, host, cap).await
    }

    /// GET an addon icon and return its bytes, at most `cap` of them. The
    /// host was vetted by the caller (`icons::fetch`), every redirect hop by
    /// the client's policy, and where the body comes from is checked again
    /// here. Never cached: the GUI keeps the decoded picture instead.
    pub async fn get_image(&self, url: &str, cap: usize) -> Result<Bytes, HttpError> {
        let parsed = parse_https(url)?;
        let host = parsed.host_str().unwrap_or("server").to_string();
        let resp = self
            .images
            .get(parsed)
            .header("Accept", "image/png, image/jpeg")
            .send()
            .await?;
        let landed = resp.url().host_str().unwrap_or_default();
        if resp.url().scheme() != "https" || !crate::icons::is_allowed_host(landed) {
            return Err(HttpError::UntrustedRedirect(landed.to_string()));
        }
        read_capped(resp, host, cap).await
    }
}

/// Check the status, then read the body, refusing it as soon as it grows
/// past `cap`: a declared length is checked up front, and a chunked or
/// compressed body is counted while it streams, so nothing larger than
/// `cap` is ever buffered.
async fn read_capped(mut resp: reqwest::Response, host: String, cap: usize) -> Result<Bytes, HttpError> {
    let status = resp.status();
    if !status.is_success() {
        return Err(HttpError::Status {
            host,
            status: status.as_u16(),
        });
    }
    if resp.content_length().is_some_and(|len| len > cap as u64) {
        return Err(HttpError::BodyTooLarge);
    }
    let mut body = Vec::with_capacity(resp.content_length().map_or(0, |l| l as usize));
    while let Some(chunk) = resp.chunk().await? {
        if body.len() + chunk.len() > cap {
            return Err(HttpError::BodyTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(body))
}

/// Parse `url` and refuse anything but `https://`, before any request.
fn parse_https(url: &str) -> Result<Url, HttpError> {
    let parsed = Url::parse(url)?;
    if parsed.scheme() != "https" {
        return Err(HttpError::InsecureUrl(url.to_string()));
    }
    Ok(parsed)
}

/// The form of `raw` that may be handed to the OS browser, or `None`.
///
/// Links come from `.toc` files and API answers, i.e. untrusted input, and
/// the OS opens whatever it is given: a `file:` path, a `javascript:` URL or
/// a custom protocol handler. Only `https://` with a host passes. A URL with
/// credentials (`https://github.com@evil.example/`) is refused too: it reads
/// as one site and opens another. The normalised serialisation is returned
/// so the browser gets exactly what was checked.
pub fn browser_url(raw: &str) -> Option<String> {
    let url = Url::parse(raw.trim()).ok()?;
    let has_host = url.host_str().is_some_and(|h| !h.is_empty());
    let has_userinfo = !url.username().is_empty() || url.password().is_some();
    (url.scheme() == "https" && has_host && !has_userinfo).then(|| url.into())
}

fn build(redirects: Policy, timeout: Duration) -> Client {
    Client::builder()
        .user_agent(USER_AGENT)
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .redirect(redirects)
        // No `cookies` feature enabled: the client never stores cookies.
        .build()
        .expect("failed to build HTTP client")
}

/// Whether to follow a redirect to `next`, given the URLs already visited
/// (`previous[0]` is the one requested): HTTPS only, at most
/// [`MAX_REDIRECTS`], and the new host must pass `allowed`. Decided before
/// the request leaves, so a refused host is never contacted.
///
/// Downloads checked only the first URL before this: a provider CDN
/// redirecting elsewhere would have been followed blindly.
pub fn redirect_verdict(previous: &[Url], next: &Url, allowed: impl Fn(&str) -> bool) -> Result<(), String> {
    if previous.len() > MAX_REDIRECTS {
        return Err("too many redirects".into());
    }
    if next.scheme() != "https" {
        return Err("refusing a redirect to a non-HTTPS address".into());
    }
    match next.host_str() {
        Some(host) if allowed(host) => Ok(()),
        Some(host) => Err(format!("refusing a redirect to {host}")),
        None => Err("refusing a redirect without a host".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(list: &[&str]) -> Vec<Url> {
        list.iter().map(|u| Url::parse(u).unwrap()).collect()
    }

    #[test]
    fn download_redirects_stay_on_trusted_hosts() {
        let trusted = crate::download::is_trusted_host;
        let from =
            urls(&["https://github.com/BigWigsMods/BigWigs/releases/download/v427.7/BigWigs-v427.7.zip"]);
        // What GitHub really does.
        let asset =
            Url::parse("https://release-assets.githubusercontent.com/github-production-release-asset/1")
                .unwrap();
        assert!(redirect_verdict(&from, &asset, trusted).is_ok());
        let evil = Url::parse("https://evil.example/x.zip").unwrap();
        assert_eq!(
            redirect_verdict(&from, &evil, trusted),
            Err("refusing a redirect to evil.example".into())
        );
        let plain = Url::parse("http://github.com/x.zip").unwrap();
        assert!(redirect_verdict(&from, &plain, trusted).is_err());
        let ping_pong = urls(&[
            "https://github.com/a",
            "https://github.com/b",
            "https://github.com/c",
            "https://github.com/d",
        ]);
        assert!(redirect_verdict(&ping_pong, &asset, trusted).is_err());
    }

    #[test]
    fn api_redirects_stay_on_the_same_host() {
        let from = urls(&["https://api.github.com/repos/old/name/releases"]);
        let same = |h: &str| h.eq_ignore_ascii_case("api.github.com");
        let moved = Url::parse("https://api.github.com/repositories/1/releases").unwrap();
        assert!(redirect_verdict(&from, &moved, same).is_ok());
        let other = Url::parse("https://github.com/old/name").unwrap();
        assert!(redirect_verdict(&from, &other, same).is_err());
    }

    #[tokio::test]
    async fn refuses_plain_http() {
        let http = Http::new();
        let err = http
            .get_json::<serde_json::Value>("http://example.com/x", &[])
            .await
            .unwrap_err();
        assert!(matches!(err, HttpError::InsecureUrl(_)));
        let err = http
            .get_json_uncached::<serde_json::Value>("http://example.com/x", &[])
            .await
            .unwrap_err();
        assert!(matches!(err, HttpError::InsecureUrl(_)));
    }

    #[test]
    fn browser_urls_are_https_with_a_host_and_no_credentials() {
        assert_eq!(
            browser_url("https://github.com/WeakAuras/WeakAuras2/releases").as_deref(),
            Some("https://github.com/WeakAuras/WeakAuras2/releases")
        );
        // Normalised: host lowercased, empty path gets its slash.
        assert_eq!(
            browser_url(" https://ZygorGuides.com ").as_deref(),
            Some("https://zygorguides.com/")
        );
        assert_eq!(browser_url("http://zygorguides.com/"), None);
        assert_eq!(browser_url("file:///C:/Windows/System32/calc.exe"), None);
        assert_eq!(browser_url("javascript:alert(1)"), None);
        assert_eq!(browser_url("ms-settings:privacy"), None);
        assert_eq!(browser_url("https://github.com@evil.example/"), None);
        assert_eq!(browser_url("https://user:pass@github.com/"), None);
        // WHATWG parsing takes the first path segment as the host: what is
        // opened is the normalised form, which names that host plainly.
        assert_eq!(browser_url("https:///nohost"), Some("https://nohost/".into()));
        assert_eq!(browser_url("not a url"), None);
        assert_eq!(browser_url(""), None);
    }
}
