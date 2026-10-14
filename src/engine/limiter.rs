//! Speed limits: a token bucket shared by the connections it limits (one
//! for everything, one per download). A rate of 0 is unlimited.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct Limiter {
    rate: AtomicU64,
    bucket: Mutex<Bucket>,
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Default for Limiter {
    fn default() -> Self {
        Limiter::new(0)
    }
}

impl Limiter {
    pub fn new(bytes_per_sec: u64) -> Limiter {
        Limiter { rate: AtomicU64::new(bytes_per_sec), bucket: Mutex::new(Bucket { tokens: 0.0, at: Instant::now() }) }
    }

    pub fn rate(&self) -> u64 {
        self.rate.load(Ordering::Relaxed)
    }

    pub fn set_rate(&self, bytes_per_sec: u64) {
        self.rate.store(bytes_per_sec, Ordering::Relaxed);
    }

    /// Wait until up to `want` bytes may pass and return how many (at least
    /// 1), or 0 once `stop` is set. Unlimited returns `want` at once.
    pub fn take(&self, want: usize, stop: &AtomicBool) -> usize {
        loop {
            let rate = self.rate();
            if rate == 0 {
                return want;
            }
            let wait = {
                let mut b = self.bucket.lock().unwrap();
                let now = Instant::now();
                // A burst of at most a tenth of a second (but a few KB at least).
                let cap = (rate as f64 / 10.0).max(4096.0);
                b.tokens = (b.tokens + now.duration_since(b.at).as_secs_f64() * rate as f64).min(cap);
                b.at = now;
                if b.tokens >= 1.0 {
                    let grant = (want as f64).min(b.tokens).floor().max(1.0);
                    b.tokens -= grant;
                    return grant as usize;
                }
                Duration::from_secs_f64(((1024.0f64).min(want as f64) - b.tokens) / rate as f64)
            };
            if stop.load(Ordering::Relaxed) {
                return 0;
            }
            // Short naps: a changed rate or a stop takes effect quickly.
            std::thread::sleep(wait.clamp(Duration::from_millis(2), Duration::from_millis(50)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_passes_everything() {
        let l = Limiter::new(0);
        assert_eq!(l.take(65536, &AtomicBool::new(false)), 65536);
    }

    #[test]
    fn limited_keeps_the_rate() {
        let l = Limiter::new(200_000);
        let stop = AtomicBool::new(false);
        let t = Instant::now();
        let mut got = 0;
        while got < 100_000 {
            got += l.take(16384, &stop);
        }
        let secs = t.elapsed().as_secs_f64();
        // 100 KB at 200 KB/s: about half a second (the first burst is free).
        assert!((0.3..1.0).contains(&secs), "took {secs} s");
    }

    #[test]
    fn stop_ends_the_wait() {
        let l = Limiter::new(1);
        let stop = AtomicBool::new(true);
        assert_eq!(l.take(4096, &stop), 0);
    }
}
