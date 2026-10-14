//! Checksums of finished files: MD5, SHA-1, SHA-256, SHA-512.

use std::io::Read;
use std::path::Path;

use md5::Md5;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Algo {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl Algo {
    pub const ALL: [Algo; 4] = [Algo::Md5, Algo::Sha1, Algo::Sha256, Algo::Sha512];

    pub fn label(self) -> &'static str {
        match self {
            Algo::Md5 => "MD5",
            Algo::Sha1 => "SHA-1",
            Algo::Sha256 => "SHA-256",
            Algo::Sha512 => "SHA-512",
        }
    }

    /// The algorithm a hex digest of this length comes from.
    fn by_len(n: usize) -> Option<Algo> {
        match n {
            32 => Some(Algo::Md5),
            40 => Some(Algo::Sha1),
            64 => Some(Algo::Sha256),
            128 => Some(Algo::Sha512),
            _ => None,
        }
    }
}

/// A checksum to compare a file with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Expected {
    pub algo: Algo,
    /// Lower-case hex.
    pub hex: String,
}

impl Expected {
    /// "sha256:ab12…", "SHA-256 ab12…" or just the hex digest (the
    /// algorithm follows from its length).
    pub fn parse(text: &str) -> Option<Expected> {
        let t = text.trim();
        let hex = t.rsplit([':', ' ', '=']).next()?.trim().to_ascii_lowercase();
        if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let algo = Algo::by_len(hex.len())?;
        Some(Expected { algo, hex })
    }
}

/// Digests of `path` for `algos`, in one read (blocking). `progress` gets
/// the bytes read so far.
pub fn compute(path: &Path, algos: &[Algo], mut progress: impl FnMut(u64)) -> std::io::Result<Vec<(Algo, String)>> {
    let mut f = std::fs::File::open(path)?;
    let (mut md5, mut sha1, mut sha256, mut sha512) = (Md5::new(), Sha1::new(), Sha256::new(), Sha512::new());
    let mut buf = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        for a in algos {
            match a {
                Algo::Md5 => md5.update(chunk),
                Algo::Sha1 => sha1.update(chunk),
                Algo::Sha256 => sha256.update(chunk),
                Algo::Sha512 => sha512.update(chunk),
            }
        }
        total += n as u64;
        progress(total);
    }
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    Ok(algos
        .iter()
        .map(|&a| {
            let h = match a {
                Algo::Md5 => hex(&md5.clone().finalize()),
                Algo::Sha1 => hex(&sha1.clone().finalize()),
                Algo::Sha256 => hex(&sha256.clone().finalize()),
                Algo::Sha512 => hex(&sha512.clone().finalize()),
            };
            (a, h)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digests_of_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("abc");
        std::fs::write(&p, b"abc").unwrap();
        let d = compute(&p, &Algo::ALL, |_| {}).unwrap();
        assert_eq!(d[0].1, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(d[1].1, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(d[2].1, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn expected_forms() {
        let sha = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(Expected::parse(&format!("sha256:{sha}")).unwrap().algo, Algo::Sha256);
        assert_eq!(Expected::parse(&format!("SHA-256 {}", sha.to_uppercase())).unwrap().hex, sha);
        assert_eq!(Expected::parse("900150983cd24fb0d6963f7d28e17f72").unwrap().algo, Algo::Md5);
        assert_eq!(Expected::parse("xyz"), None);
        assert_eq!(Expected::parse("abcd"), None);
    }
}
