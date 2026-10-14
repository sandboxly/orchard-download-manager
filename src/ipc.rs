//! One running app: a second start hands its links to the first and quits.
//! The first listens on a local TCP port; the port and a random token are
//! in `instance.json` in the config folder (only this user can read it).
//! A message is one JSON line: `{"token": "…", "links": ["…"]}`, from the
//! browser extension's host also with `"items"` (links with the request
//! headers the browser would send) and `"browser": true`.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Instance {
    port: u16,
    token: String,
    pid: u32,
}

#[derive(Serialize, Deserialize)]
struct Message {
    token: String,
    links: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    items: Vec<Link>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    browser: bool,
}

/// A link with what goes with it (from the browser).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub url: String,
    /// Request headers: cookies, the referring page, the browser's name.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// The file name the browser chose.
    #[serde(default)]
    pub name: Option<String>,
}

impl Link {
    pub fn plain(url: String) -> Link {
        Link { url, ..Link::default() }
    }
}

/// What another start or the browser hands over.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Handoff {
    /// From the command line (any text with links in it).
    pub links: Vec<String>,
    /// From the browser: exact links.
    pub items: Vec<Link>,
    pub browser: bool,
}

fn instance_file(dir: &Path) -> PathBuf {
    dir.join("instance.json")
}

/// Hand `links` to a running app. `Ok(true)`: it took them (also with no
/// links: it comes to the front).
pub fn send(dir: &Path, links: &[String]) -> bool {
    send_handoff(dir, &Handoff { links: links.to_vec(), ..Handoff::default() })
}

/// Hand links (with their headers) to a running app.
pub fn send_handoff(dir: &Path, h: &Handoff) -> bool {
    let Ok(text) = std::fs::read_to_string(instance_file(dir)) else { return false };
    let Ok(inst) = serde_json::from_str::<Instance>(&text) else { return false };
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], inst.port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let msg = Message { token: inst.token, links: h.links.clone(), items: h.items.clone(), browser: h.browser };
    let Ok(line) = serde_json::to_string(&msg) else { return false };
    if writeln!(s, "{line}").is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply).is_ok() && reply.trim() == "ok"
}

/// Listen for other starts; `on_links` runs on the listener's thread.
pub fn listen(dir: &Path, on_links: impl Fn(Handoff) + Send + 'static) -> std::io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let token = random_token();
    let inst = Instance { port: listener.local_addr()?.port(), token: token.clone(), pid: std::process::id() };
    std::fs::create_dir_all(dir)?;
    write_private(&instance_file(dir), &serde_json::to_string(&inst).unwrap_or_default())?;
    std::thread::Builder::new().name("instance".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            let mut reader = BufReader::new(&stream);
            if reader.read_line(&mut line).is_err() || line.len() > 1_000_000 {
                continue;
            }
            match serde_json::from_str::<Message>(&line) {
                Ok(m) if m.token == token => {
                    let _ = (&stream).write_all(b"ok\n");
                    on_links(Handoff { links: m.links, items: m.items, browser: m.browser });
                }
                _ => {
                    let _ = (&stream).write_all(b"no\n");
                }
            }
        }
    })?;
    Ok(())
}

/// Remove the instance file if it is ours (on quit).
pub fn forget(dir: &Path) {
    let file = instance_file(dir);
    if let Ok(text) = std::fs::read_to_string(&file)
        && serde_json::from_str::<Instance>(&text).is_ok_and(|i| i.pid == std::process::id())
    {
        let _ = std::fs::remove_file(file);
    }
}

/// Write a file only this user can read (through a temporary file).
pub(crate) fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        o.open(&tmp)?.write_all(text.as_bytes())?;
    }
    std::fs::rename(tmp, path)
}

fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    (0..4)
        .map(|i| {
            let mut h = RandomState::new().build_hasher();
            h.write_u64(i);
            h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
            format!("{:016x}", h.finish())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_second_start_hands_over_its_links() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!send(dir.path(), &["https://x.org/a".into()]), "nobody listens yet");
        let (tx, rx) = mpsc::channel();
        listen(dir.path(), move |l| tx.send(l).unwrap()).unwrap();
        assert!(send(dir.path(), &["https://x.org/a".into()]));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap().links, ["https://x.org/a"]);

        let link = Link { url: "https://x.org/b".into(), headers: vec![("Cookie".into(), "a=1".into())], name: Some("b.zip".into()) };
        let h = Handoff { links: Vec::new(), items: vec![link], browser: true };
        assert!(send_handoff(dir.path(), &h));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), h);
        forget(dir.path());
        assert!(!instance_file(dir.path()).exists());
    }

    #[test]
    fn a_wrong_token_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel::<Handoff>();
        listen(dir.path(), move |l| tx.send(l).unwrap()).unwrap();
        let text = std::fs::read_to_string(instance_file(dir.path())).unwrap();
        let mut inst: Instance = serde_json::from_str(&text).unwrap();
        inst.token = "guess".into();
        std::fs::write(instance_file(dir.path()), serde_json::to_string(&inst).unwrap()).unwrap();
        assert!(!send(dir.path(), &["https://x.org/a".into()]));
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn messages_from_older_versions_still_read() {
        let m: Message = serde_json::from_str(r#"{"token":"t","links":["https://x.org/a"]}"#).unwrap();
        assert!(m.items.is_empty() && !m.browser);
    }
}
