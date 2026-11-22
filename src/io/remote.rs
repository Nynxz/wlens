//! HTTP for URL items and `wlens.fetch`. Runs on loader worker threads only.
//!
//! Downloaded images are cached under `$XDG_CACHE_HOME/wlens/remote/<md5(url)>`
//! so revisiting a gallery doesn't refetch, and thumbnails can be generated
//! from a real file.
//!
//! Scripts can register headers per URL prefix (`wlens.http_headers`), e.g. an
//! API key for a self-hosted media server. They're sent with *every* request
//! to that origin and path prefix — API calls, image downloads and thumbnails — so secrets
//! never need to live in item URLs.

use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

use std::time::Duration;
use url::Url;

/// Refuse anything bigger than this (bytes) for an image download.
const MAX_IMAGE: u64 = 256 * 1024 * 1024;
/// ...and for `wlens.fetch` text bodies (JSON APIs, HTML).
const MAX_TEXT: u64 = 32 * 1024 * 1024;

/// The proxy everything goes through: `wlens.opt.proxy` ("http://host:port",
/// "socks5://host:port"), else the standard *_PROXY environment variables.
fn proxy_setting() -> &'static RwLock<Option<String>> {
    static PROXY: OnceLock<RwLock<Option<String>>> = OnceLock::new();
    PROXY.get_or_init(Default::default)
}

fn agent_slot() -> &'static RwLock<Option<ureq::Agent>> {
    static AGENT: OnceLock<RwLock<Option<ureq::Agent>>> = OnceLock::new();
    AGENT.get_or_init(Default::default)
}

/// Set (or with an empty string, clear) the proxy. Later requests use it.
pub fn set_proxy(url: &str) -> Result<(), String> {
    let url = url.trim();
    if !url.is_empty() {
        ureq::Proxy::new(url).map_err(|e| format!("proxy '{url}': {e}"))?;
    }
    *proxy_setting().write().unwrap() = (!url.is_empty()).then(|| url.to_string());
    *agent_slot().write().unwrap() = None; // rebuilt on the next request
    Ok(())
}

/// The proxy in effect, if any (for other HTTP clients, e.g. GStreamer).
#[cfg(feature = "video")]
pub fn proxy() -> Option<String> {
    proxy_setting().read().unwrap().clone().or_else(|| {
        [
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
            "HTTP_PROXY",
            "http_proxy",
        ]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
    })
}

fn agent() -> ureq::Agent {
    if let Some(a) = agent_slot().read().unwrap().as_ref() {
        return a.clone();
    }
    let explicit = proxy_setting().read().unwrap().clone();
    let a: ureq::Agent = {
        let b = ureq::Agent::config_builder();
        // An explicit proxy wins; otherwise ureq reads *_PROXY from the environment.
        let b = match explicit.as_deref().map(ureq::Proxy::new) {
            Some(Ok(p)) => b.proxy(Some(p)),
            Some(Err(e)) => {
                log::warn!("proxy: {e}");
                b
            }
            None => b,
        };
        b.max_redirects(0)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_resolve(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_per_read(Some(Duration::from_secs(30)))
            .timeout_per_write(Some(Duration::from_secs(30)))
            .user_agent(concat!("wlens/", env!("CARGO_PKG_VERSION")))
            // We look at the status ourselves so scripts can read error bodies
            // (GraphQL and REST APIs explain failures in the body).
            .http_status_as_error(false)
            .build()
            .into()
    };
    *agent_slot().write().unwrap() = Some(a.clone());
    a
}

type Headers = Vec<(String, String)>;

fn registry() -> &'static RwLock<Vec<(String, Headers)>> {
    static REG: OnceLock<RwLock<Vec<(String, Headers)>>> = OnceLock::new();
    REG.get_or_init(Default::default)
}

/// Register headers for an exact HTTP origin and an optional path prefix.
pub fn set_headers(prefix: &str, headers: Headers) -> anyhow::Result<()> {
    let scope = http_url(prefix)?;
    let mut reg = registry().write().unwrap();
    reg.retain(|(p, _)| p != scope.as_str());
    if !headers.is_empty() {
        reg.push((scope.to_string(), headers));
    }
    Ok(())
}

fn http_url(value: &str) -> anyhow::Result<Url> {
    let mut url = Url::parse(value)?;
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https"),
        "expected an HTTP(S) URL"
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "URL credentials are not supported; use HTTP headers"
    );
    url.set_fragment(None);
    Ok(url)
}

fn in_scope(scope: &Url, url: &Url) -> bool {
    scope.origin() == url.origin() && url.path().starts_with(scope.path())
}

/// The registered headers for a URL (for clients other than ours, e.g. the
/// video player's HTTP source).
#[cfg(feature = "video")]
pub fn registered_headers(url: &str) -> Headers {
    Url::parse(url).map(|u| headers_for(&u)).unwrap_or_default()
}

fn headers_for(url: &Url) -> Headers {
    registry()
        .read()
        .unwrap()
        .iter()
        .filter(|(p, _)| Url::parse(p).is_ok_and(|scope| in_scope(&scope, url)))
        .flat_map(|(_, h)| h.clone())
        .collect()
}

/// Build every redirect as a fresh request. Per-request headers and bodies
/// never cross origins; registered headers are looked up for each target.
fn request(req: &Request) -> anyhow::Result<ureq::http::Response<ureq::Body>> {
    let initial = http_url(&req.url)?;
    let mut url = initial.clone();
    let mut method = req.method.to_ascii_uppercase();
    let mut body = req.body.clone();
    let mut forward_headers = true;
    for hop in 0..=10 {
        let mut headers = headers_for(&url);
        if forward_headers {
            for (k, v) in &req.headers {
                headers.retain(|(name, _)| !name.eq_ignore_ascii_case(k));
                headers.push((k.clone(), v.clone()));
            }
        }
        // Let the transport calculate framing and the Host for this target.
        headers.retain(|(k, _)| {
            !matches!(
                k.to_ascii_lowercase().as_str(),
                "host" | "content-length" | "transfer-encoding"
            )
        });
        macro_rules! with_headers {
            ($builder:expr) => {{
                let mut builder = $builder;
                for (k, v) in &headers {
                    builder = builder.header(k.as_str(), v.as_str());
                }
                builder
            }};
        }
        let a = agent();
        let res = match method.as_str() {
            "GET" => with_headers!(a.get(url.as_str())).call()?,
            "DELETE" => with_headers!(a.delete(url.as_str())).call()?,
            "POST" => with_headers!(a.post(url.as_str())).send(body.as_deref().unwrap_or(""))?,
            "PUT" => with_headers!(a.put(url.as_str())).send(body.as_deref().unwrap_or(""))?,
            "PATCH" => with_headers!(a.patch(url.as_str())).send(body.as_deref().unwrap_or(""))?,
            m => anyhow::bail!("unsupported method {m}"),
        };
        let status = res.status().as_u16();
        if !req.follow || !matches!(status, 301 | 302 | 303 | 307 | 308) {
            return Ok(res);
        }
        let Some(location) = res.headers().get("location") else {
            return Ok(res);
        };
        anyhow::ensure!(hop < 10, "too many HTTP redirects");
        let next = http_url(url.join(location.to_str()?)?.as_str())?;
        anyhow::ensure!(
            url.scheme() != "https" || next.scheme() == "https",
            "refusing HTTPS downgrade"
        );
        let cross_origin = url.origin() != next.origin();
        if status == 303 || (matches!(status, 301 | 302) && method == "POST") {
            method = "GET".into();
            body = None;
        }
        if cross_origin && (body.is_some() || method != "GET") {
            anyhow::bail!("refusing to forward request body or method to another origin");
        }
        // Once removed, credentials stay removed even if a later hop returns.
        forward_headers &= !cross_origin;
        url = next;
    }
    unreachable!()
}

fn get(url: &str) -> anyhow::Result<ureq::http::Response<ureq::Body>> {
    request(&Request {
        follow: true,
        url: url.into(),
        method: "GET".into(),
        headers: vec![],
        body: None,
    })
}

fn cache_lock(path: &std::path::Path) -> Arc<Mutex<()>> {
    type Locks = std::collections::HashMap<PathBuf, Weak<Mutex<()>>>;
    static LOCKS: OnceLock<Mutex<Locks>> = OnceLock::new();
    let mut locks = LOCKS.get_or_init(Default::default).lock().unwrap();
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(path.to_path_buf(), Arc::downgrade(&lock));
    lock
}

pub fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("wlens").join("remote"))
}

/// Where a URL's download lives (whether or not it exists yet).
pub fn cached_path(url: &str) -> Option<PathBuf> {
    cache_dir().map(|d| d.join(format!("{:x}", md5::compute(url.as_bytes()))))
}

/// Mark a cache file as just used (for least-recently-used pruning).
fn touch(path: &std::path::Path) {
    if let Ok(f) = std::fs::File::options().append(true).open(path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

/// Download `url` into the cache (once) and return the local file.
pub fn fetch_to_cache(url: &str) -> anyhow::Result<PathBuf> {
    let path = cached_path(url).ok_or_else(|| anyhow::anyhow!("no cache directory"))?;
    fetch_image_at(url, path)
}

fn fetch_image_at(url: &str, path: PathBuf) -> anyhow::Result<PathBuf> {
    let lock = cache_lock(&path);
    let _guard = lock.lock().unwrap();
    if path.exists() {
        touch(&path);
        return Ok(path);
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    let t = std::time::Instant::now();
    let mut res = get(url)?;
    let status = res.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {status} for {url}");
    }
    // A video or a web page isn't going to decode: stop before reading it all.
    if let Some(ct) = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
    {
        let ct = ct.to_ascii_lowercase();
        if ct.starts_with("video/") || ct.starts_with("text/html") {
            anyhow::bail!("{url} is {ct}, not an image");
        }
    }
    let mut bytes = Vec::new();
    res.body_mut()
        .with_config()
        .limit(MAX_IMAGE)
        .reader()
        .read_to_end(&mut bytes)?;
    // Write-then-rename so a half download never looks complete.
    // Reject successful login/error pages before they poison the cache.
    image::guess_format(&bytes)?;
    let mut tmp = super::atomic::TempFile::new(&path)?;
    std::io::Write::write_all(&mut tmp.file, &bytes)?;
    tmp.persist(&path)?;
    log::debug!(
        "downloaded {url} ({} KiB) in {:?}",
        bytes.len() / 1024,
        t.elapsed()
    );
    Ok(path)
}

/// Download `url` to `dest` (a script's `wlens.download`), streaming, with
/// `progress(bytes_so_far, total_if_known)` called now and then. Written to a
/// temporary file and renamed at the end, so a partial file never looks done.
pub fn download(
    url: &str,
    dest: &std::path::Path,
    mut progress: impl FnMut(u64, Option<u64>),
) -> anyhow::Result<u64> {
    use std::io::Write;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Already viewed (so in the cache)? Copy it instead of asking again.
    if let Some(cached) = cached_path(url).filter(|p| p.exists()) {
        touch(&cached);
        let mut tmp = super::atomic::TempFile::new(dest)?;
        let n = std::io::copy(&mut std::fs::File::open(&cached)?, &mut tmp.file)?;
        tmp.persist(dest)?;
        progress(n, Some(n));
        log::debug!("download {url}: copied from cache");
        return Ok(n);
    }
    let mut res = get(url)?;
    let status = res.status();
    if !status.is_success() {
        // Rate limited: pass on how long the server wants us to wait.
        let retry = res
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        match retry {
            Some(secs) => anyhow::bail!("HTTP {status} (retry after {secs}s)"),
            None => anyhow::bail!("HTTP {status}"),
        }
    }
    // A web page where a file was expected: a login, an error page, or a
    // browser check in front of the site. Don't save it as the file.
    let html = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/html"));
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    if html && !path.ends_with(".html") && !path.ends_with(".htm") {
        anyhow::bail!("got a web page instead of the file (a login or browser check?)");
    }
    let total = res
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let mut tmp = super::atomic::TempFile::new(dest)?;
    (|| -> anyhow::Result<u64> {
        let out = &mut tmp.file;
        let mut reader = res.body_mut().with_config().limit(u64::MAX).reader();
        let mut buf = vec![0u8; 64 * 1024];
        let (mut done, mut last) = (0u64, std::time::Instant::now());
        progress(0, total);
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            done += n as u64;
            if last.elapsed() > Duration::from_millis(100) {
                last = std::time::Instant::now();
                progress(done, total);
            }
        }
        out.sync_all()?;
        tmp.persist(dest)?;
        progress(done, total.or(Some(done)));
        Ok(done)
    })()
}

/// A video URL into the cache, if it's small enough to be worth it:
/// `Ok(Some(path))` cached (or already was), `Ok(None)` too big or of unknown
/// size (stream it instead). One request either way.
#[cfg(feature = "video")]
pub fn cache_video(url: &str, max_bytes: u64) -> anyhow::Result<Option<PathBuf>> {
    let path = cached_path(url).ok_or_else(|| anyhow::anyhow!("no cache directory"))?;
    let lock = cache_lock(&path);
    let _guard = lock.lock().unwrap();
    if path.exists() {
        touch(&path);
        return Ok(Some(path));
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    let mut res = get(url)?;
    let status = res.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {status}");
    }
    let len = res
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if len.is_none_or(|l| l > max_bytes) {
        return Ok(None); // dropping the response closes it
    }
    let mut tmp = super::atomic::TempFile::new(&path)?;
    let mut reader = res.body_mut().with_config().limit(max_bytes).reader();
    std::io::copy(&mut reader, &mut tmp.file)?;
    tmp.persist(&path)?;
    Ok(Some(path))
}

/// Keep the download cache under `max_bytes`: delete the least recently
/// used files (cache hits refresh a file's time) until it fits.
pub fn prune_cache(max_bytes: u64) {
    let Some(dir) = cache_dir() else { return };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.len() == 32 && n.bytes().all(|b| b.is_ascii_hexdigit()))
        })
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| {
                (
                    m.modified().unwrap_or(std::time::UNIX_EPOCH),
                    m.len(),
                    e.path(),
                )
            })
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= max_bytes {
        return;
    }
    files.sort_by_key(|f| f.0); // oldest first
    let (mut n, before) = (0, total);
    for (_, len, path) in files {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
            n += 1;
        }
    }
    log::info!(
        "remote cache: removed {n} files, {} → {} MiB",
        before >> 20,
        total >> 20
    );
}

/// What a script's fetch delivers: (status, body, headers), or an error message.
pub type FetchResult = Result<(u16, String, Vec<(String, String)>), String>;

/// A script's HTTP request (`wlens.fetch`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub url: String,
    pub method: String,
    pub headers: Headers,
    pub body: Option<String>,
    /// Follow redirects (default). Off: return the 3xx itself — e.g. to read
    /// the cookies a challenge sets on its redirect.
    pub follow: bool,
}

pub struct Response {
    pub status: u16,
    pub body: String,
    /// Response headers, names lowercased (e.g. `last-modified`).
    pub headers: Vec<(String, String)>,
}

pub fn fetch(req: &Request) -> anyhow::Result<Response> {
    let mut res = request(req)?;
    let status = res.status().as_u16();
    let resp_headers: Vec<(String, String)> = res
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            Some((
                k.as_str().to_ascii_lowercase(),
                v.to_str().ok()?.to_string(),
            ))
        })
        .collect();
    let body = res
        .body_mut()
        .with_config()
        .limit(MAX_TEXT)
        .read_to_string()?;
    Ok(Response {
        status,
        body,
        headers: resp_headers,
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;
    use std::net::TcpListener;

    fn server(response: String) -> (String, std::thread::JoinHandle<Option<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let thread = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            return None;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                if stream.read(&mut byte).unwrap() == 0 {
                    break;
                }
                bytes.push(byte[0]);
            }
            stream.write_all(response.as_bytes()).unwrap();
            Some(String::from_utf8(bytes).unwrap().to_ascii_lowercase())
        });
        (url, thread)
    }

    fn success() -> String {
        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".into()
    }

    #[test]
    fn successful_html_response_does_not_poison_image_cache() {
        let (source, server) = server(
            "HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\n<html></html>"
                .into(),
        );
        let dir = std::env::temp_dir().join(format!(
            "wlens-invalid-media-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        let path = dir.join("image");
        assert!(fetch_image_at(&source, path.clone()).is_err());
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        server.join().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scopes_require_exact_origin() {
        let scope = http_url("http://media").unwrap();
        assert!(in_scope(
            &scope,
            &http_url("http://MEDIA:80/graphql").unwrap()
        ));
        for other in [
            "http://media.evil.net/graphql",
            "http://media:81/graphql",
            "https://media/graphql",
        ] {
            assert!(!in_scope(&scope, &http_url(other).unwrap()), "{other}");
        }
        assert!(http_url("http://media@evil.net").is_err());
        assert!(http_url("file:///tmp/image").is_err());
        assert!(in_scope(
            &http_url("http://media/api/").unwrap(),
            &http_url("http://media/api/image").unwrap()
        ));
        assert!(!in_scope(
            &http_url("http://media/api/").unwrap(),
            &http_url("http://media/other/image").unwrap()
        ));
    }

    #[test]
    fn redirect_recomputes_registered_headers_and_drops_request_secrets() {
        let (target, received) = server(success());
        let (source, first) = server(format!(
            "HTTP/1.1 302 Found\r\nLocation: {target}/image\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        ));
        set_headers(&source, vec![("ApiKey".into(), "source-secret".into())]).unwrap();
        set_headers(&target, vec![("X-Target".into(), "target-only".into())]).unwrap();
        let response = fetch(&Request {
            follow: true,
            url: source.clone(),
            method: "GET".into(),
            headers: vec![("X-Secret".into(), "private".into())],
            body: None,
        })
        .unwrap();
        assert_eq!(response.body, "ok");
        let initial = first.join().unwrap().unwrap();
        assert!(initial.contains("apikey: source-secret"));
        let redirected = received.join().unwrap().unwrap();
        assert!(!redirected.contains("source-secret"));
        assert!(!redirected.contains("x-secret"));
        assert!(redirected.contains("x-target: target-only"));
        set_headers(&source, vec![]).unwrap();
        set_headers(&target, vec![]).unwrap();
    }

    #[test]
    fn cross_origin_redirect_does_not_replay_post_body() {
        let (source, first) = server("HTTP/1.1 307 Temporary Redirect\r\nLocation: http://127.0.0.1:1/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into());
        let result = fetch(&Request {
            follow: true,
            url: source,
            method: "POST".into(),
            headers: vec![],
            body: Some("secret".into()),
        });
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("refusing to forward")
        );
        first.join().unwrap();
    }

    #[test]
    fn relative_redirect_retains_same_origin_headers() {
        // One origin handles both requests, including ../ and query resolution.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let worker = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            for step in 0..2 {
                let mut socket = loop {
                    match listener.accept() {
                        Ok((s, _)) => break s,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < deadline);
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut b = [0];
                    assert_eq!(socket.read(&mut b).unwrap(), 1);
                    request.push(b[0]);
                }
                let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
                assert!(request.contains("apikey: same-secret"));
                if step == 0 {
                    socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: ../image?size=1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                } else {
                    assert!(request.starts_with("get /image?size=1 "));
                    socket.write_all(success().as_bytes()).unwrap();
                }
            }
        });
        set_headers(&base, vec![("ApiKey".into(), "same-secret".into())]).unwrap();
        assert_eq!(
            fetch(&Request {
                follow: true,
                url: format!("{base}/api/start"),
                method: "GET".into(),
                headers: vec![],
                body: None
            })
            .unwrap()
            .body,
            "ok"
        );
        worker.join().unwrap();
        set_headers(&base, vec![]).unwrap();
    }
}
