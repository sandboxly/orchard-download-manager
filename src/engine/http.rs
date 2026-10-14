//! HTTP(S): the client built from the settings, the first request that
//! finds out a file's name, size and whether the server serves ranges, and
//! ranged requests for the connections.

use std::time::Duration;

use percent_encoding::percent_decode_str;
use ureq::tls::{RootCerts, TlsConfig};
use ureq::{Agent, Body, ResponseExt};

/// The client settings that matter to every request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetConfig {
    /// Empty: the app's own.
    pub user_agent: String,
    /// `None`: no proxy; `Some("")`: the environment's; else the proxy URL.
    pub proxy: Option<String>,
    /// Waiting for a connection or the response's headers.
    pub timeout: Duration,
}

impl Default for NetConfig {
    fn default() -> Self {
        NetConfig { user_agent: String::new(), proxy: Some(String::new()), timeout: Duration::from_secs(30) }
    }
}

pub fn default_user_agent() -> String {
    format!("OrchardDownloadManager/{}", env!("CARGO_PKG_VERSION"))
}

/// A client for `cfg`; an unusable proxy URL is an error.
pub fn agent(cfg: &NetConfig) -> Result<Agent, String> {
    let proxy = match cfg.proxy.as_deref() {
        None => None,
        Some("") => ureq::Proxy::try_from_env(),
        Some(url) => Some(ureq::Proxy::new(url).map_err(|e| format!("the proxy address doesn't work ({e})"))?),
    };
    let ua = if cfg.user_agent.trim().is_empty() { default_user_agent() } else { cfg.user_agent.trim().to_string() };
    let config = Agent::config_builder()
        .http_status_as_error(false)
        .user_agent(ua)
        .proxy(proxy)
        .max_redirects(20)
        .timeout_resolve(Some(cfg.timeout))
        .timeout_connect(Some(cfg.timeout))
        .timeout_send_request(Some(cfg.timeout))
        .timeout_recv_response(Some(cfg.timeout))
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .build();
    Ok(config.new_agent())
}

/// What the first response says about the file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Probe {
    /// After redirects.
    pub final_url: String,
    /// A file name, from the server or the URL.
    pub name: String,
    pub size: Option<u64>,
    /// The server answers range requests: several connections, resuming.
    pub resumable: bool,
    pub mime: Option<String>,
    pub validator: Validator,
}

/// What tells whether the file on the server changed since a pause.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Validator {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl Validator {
    /// Both known and different (a missing one proves nothing).
    pub fn changed(&self, now: &Validator) -> bool {
        let differs = |a: &Option<String>, b: &Option<String>| matches!((a, b), (Some(x), Some(y)) if x != y);
        differs(&self.etag, &now.etag) || differs(&self.last_modified, &now.last_modified)
    }
}

/// Extra request headers of a download (referrer, cookies from the browser).
pub type Headers = Vec<(String, String)>;

/// GET `url`, from `from` to `to` (inclusive) if given.
pub fn get(agent: &Agent, url: &str, range: Option<(u64, Option<u64>)>, headers: &Headers) -> Result<ureq::http::Response<Body>, String> {
    let mut req = agent.get(url);
    if let Some((from, to)) = range {
        let r = match to {
            Some(to) => format!("bytes={from}-{to}"),
            None => format!("bytes={from}-"),
        };
        req = req.header("Range", r);
    }
    // No transfer encodings: offsets must be bytes of the file.
    req = req.header("Accept-Encoding", "identity");
    if let Some(auth) = basic_auth(url) {
        req = req.header("Authorization", auth);
    }
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    req.call().map_err(describe_error)
}

/// The first request: a range from 0, so the answer says whether ranges work.
pub fn probe(agent: &Agent, url: &str, headers: &Headers) -> Result<Probe, String> {
    let resp = get(agent, url, Some((0, None)), headers)?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(status_reason(status));
    }
    let final_url = resp.get_uri().to_string();
    let h = resp.headers();
    let header = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
    let length = header("content-length").and_then(|v| v.trim().parse::<u64>().ok());
    let (size, resumable) = if status == 206 {
        match header("content-range").and_then(|v| parse_content_range(&v)) {
            Some((0, _, total)) => (total, total.is_some()),
            _ => (None, false),
        }
    } else {
        (length, false)
    };
    let mime = header("content-type").map(|m| m.split(';').next().unwrap_or("").trim().to_ascii_lowercase()).filter(|m| !m.is_empty());
    let from_header = header("content-disposition").and_then(|v| disposition_filename(&v));
    let name = from_header.unwrap_or_else(|| url_filename(&final_url));
    let name = with_extension_for(sanitize_filename(&name), mime.as_deref());
    let validator = Validator { etag: header("etag"), last_modified: header("last-modified") };
    Ok(Probe { final_url, name, size, resumable, mime, validator })
}

/// "bytes 0-99/1000" → (0, 99, Some(1000)); "*" sizes are `None`.
pub fn parse_content_range(v: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = v.trim().strip_prefix("bytes")?.trim_start();
    let (range, total) = rest.split_once('/')?;
    let (a, b) = range.trim().split_once('-')?;
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse().ok()?),
    };
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?, total))
}

/// The file name in a Content-Disposition header (`filename*` preferred).
pub fn disposition_filename(v: &str) -> Option<String> {
    let mut plain = None;
    for part in split_params(v) {
        let Some((k, val)) = part.split_once('=') else { continue };
        let k = k.trim().to_ascii_lowercase();
        let val = val.trim();
        if k == "filename*" {
            // charset'lang'percent-encoded
            let enc = val.splitn(3, '\'').nth(2).unwrap_or(val);
            let s = percent_decode_str(enc.trim_matches('"')).decode_utf8_lossy().into_owned();
            if !s.is_empty() {
                return Some(s);
            }
        } else if k == "filename" {
            let s = val.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(val).replace("\\\"", "\"");
            if !s.is_empty() {
                plain = Some(s);
            }
        }
    }
    plain
}

/// `;`-separated parameters, quotes respected.
fn split_params(v: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted) = (0, false);
    for (i, c) in v.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                out.push(&v[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&v[start..]);
    out
}

/// The last path segment of a URL, decoded; the host if there is none.
pub fn url_filename(url: &str) -> String {
    let Ok(u) = url::Url::parse(url) else { return "download".into() };
    let seg = u.path_segments().and_then(|mut s| s.next_back()).unwrap_or("");
    let name = percent_decode_str(seg).decode_utf8_lossy().into_owned();
    if name.is_empty() { u.host_str().map_or_else(|| "download".into(), |h| h.to_string()) } else { name }
}

/// A name that is safe as a file name on every platform.
pub fn sanitize_filename(name: &str) -> String {
    // Only the last component of a name with a path in it.
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut s: String = name
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    s = s.trim().trim_end_matches('.').trim().to_string();
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "LPT1", "LPT2", "LPT3"];
    if reserved.contains(&stem.as_str()) {
        s.insert(0, '_');
    }
    if s.is_empty() || s == "." || s == ".." {
        s = "download".into();
    }
    // Keep it under common file name limits (255 bytes), extension kept.
    if s.len() > 200 {
        let ext = std::path::Path::new(&s).extension().map(|e| e.to_string_lossy().into_owned()).filter(|e| e.len() < 16);
        let mut cut = 200 - ext.as_ref().map_or(0, |e| e.len() + 1);
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        if let Some(e) = ext {
            s = format!("{s}.{e}");
        }
    }
    s
}

/// A name without an extension gets one for well-known types (a web page
/// saved as "index" becomes "index.html").
fn with_extension_for(name: String, mime: Option<&str>) -> String {
    if std::path::Path::new(&name).extension().is_some() {
        return name;
    }
    let ext = match mime.unwrap_or("") {
        "text/html" => "html",
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "video/mp4" => "mp4",
        "audio/mpeg" => "mp3",
        "text/plain" => "txt",
        "application/json" => "json",
        _ => return name,
    };
    format!("{name}.{ext}")
}

/// `Authorization: Basic …` for "https://user:pass@host/…".
fn basic_auth(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    if u.username().is_empty() {
        return None;
    }
    let user = percent_decode_str(u.username()).decode_utf8_lossy();
    let pass = percent_decode_str(u.password().unwrap_or("")).decode_utf8_lossy();
    Some(format!("Basic {}", base64(format!("{user}:{pass}").as_bytes())))
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// An HTTP status as what went wrong, for people.
pub fn status_reason(status: u16) -> String {
    match status {
        401 | 407 => format!("the server wants a sign-in ({status})"),
        403 => "the server refused access (403) — the link may have expired".into(),
        404 | 410 => format!("the server doesn't have this file ({status})"),
        416 => "the server can't send the rest of the file (416)".into(),
        429 => "the server is limiting requests (429) — try again later".into(),
        500..=599 => format!("the server has a problem ({status})"),
        _ => format!("the server answered {status}"),
    }
}

/// A transport error as what went wrong, for people.
pub fn describe_error(e: ureq::Error) -> String {
    use ureq::Error::*;
    match e {
        Io(io) => io_net_reason(&io),
        Timeout(_) => "the server didn't answer in time".into(),
        HostNotFound => "the server's name couldn't be found — check the address".into(),
        ConnectionFailed => "couldn't connect to the server".into(),
        TooManyRedirects => "the link redirects too often".into(),
        BadUri(_) => "the address isn't a valid link".into(),
        Tls(_) | Rustls(_) => "the secure connection failed".into(),
        ConnectProxyFailed(_) | InvalidProxyUrl => "couldn't connect through the proxy".into(),
        other => other.to_string(),
    }
}

pub fn io_net_reason(e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        ConnectionRefused => "the server refused the connection".into(),
        ConnectionReset | ConnectionAborted | BrokenPipe | UnexpectedEof => "the connection broke off".into(),
        TimedOut | WouldBlock => "the connection timed out".into(),
        NotConnected | NetworkUnreachable | HostUnreachable | NetworkDown => "no network connection".into(),
        _ => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range() {
        assert_eq!(parse_content_range("bytes 0-99/1000"), Some((0, 99, Some(1000))));
        assert_eq!(parse_content_range("bytes 5-9/*"), Some((5, 9, None)));
        assert_eq!(parse_content_range("items 0-1/2"), None);
    }

    #[test]
    fn disposition() {
        assert_eq!(disposition_filename("attachment; filename=\"a b.zip\""), Some("a b.zip".into()));
        assert_eq!(disposition_filename("attachment; filename=plain.txt"), Some("plain.txt".into()));
        assert_eq!(
            disposition_filename("attachment; filename=\"fallback.txt\"; filename*=UTF-8''%E2%82%AC%20rates.txt"),
            Some("€ rates.txt".into())
        );
        assert_eq!(disposition_filename("attachment; filename=\"semi;colon.txt\""), Some("semi;colon.txt".into()));
        assert_eq!(disposition_filename("inline"), None);
    }

    #[test]
    fn names_from_urls() {
        assert_eq!(url_filename("https://x.org/a/b/My%20File.iso?x=1"), "My File.iso");
        assert_eq!(url_filename("https://x.org/"), "x.org");
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("a:b?.txt"), "a_b_.txt");
        assert_eq!(sanitize_filename("con.txt"), "_con.txt");
        assert_eq!(sanitize_filename("..."), "download");
        assert_eq!(with_extension_for("index".into(), Some("text/html")), "index.html");
    }

    #[test]
    fn auth_header() {
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(basic_auth("https://user:pass@x.org/f"), Some("Basic dXNlcjpwYXNz".into()));
        assert_eq!(basic_auth("https://x.org/f"), None);
    }

    #[test]
    fn validators() {
        let a = Validator { etag: Some("\"1\"".into()), last_modified: None };
        assert!(a.changed(&Validator { etag: Some("\"2\"".into()), last_modified: None }));
        assert!(!a.changed(&Validator::default()));
    }
}
