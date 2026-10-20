//! Short-term duplicate suppression.
//!
//! Copying the same link twice — or a chat app re-setting the
//! clipboard, which fires WM_CLIPBOARDUPDATE again — must not stack
//! up popups. `SeenCache` remembers normalized URLs for a
//! configurable window; `mark` returns true when the URL is new
//! enough to act on.
//!
//! The cache is deliberately in-memory only. Restarting ytTrayDL is
//! an obvious "ask me again" signal, and the yt-dlp download archive
//! (a separate, persistent mechanism) covers true re-downloads.

use crate::url_detect::normalize_url;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct SeenCache {
    seen: HashMap<String, Instant>,
    window: Duration,
}

impl SeenCache {
    pub fn new(window: Duration) -> Self {
        Self {
            seen: HashMap::new(),
            window,
        }
    }

    pub fn set_window(&mut self, window: Duration) {
        self.window = window;
    }

    /// Record `url`. Returns `true` if it should be acted on (not
    /// seen within the window), `false` to suppress it.
    /// A window of zero disables suppression entirely.
    pub fn mark(&mut self, url: &str) -> bool {
        self.prune();
        if self.window.is_zero() {
            return true;
        }
        let key = normalize_url(url);
        let now = Instant::now();
        if let Some(at) = self.seen.get(&key) {
            if now.duration_since(*at) < self.window {
                return false;
            }
        }
        self.seen.insert(key, now);
        true
    }

    /// Remove a URL (e.g. the user dismissed the popup and then
    /// deliberately copies the link again — a manual re-copy after
    /// dismissal is still suppressed by the window, but a download
    /// failure clears the entry so a re-copy retries immediately).
    pub fn forget(&mut self, url: &str) {
        self.seen.remove(&normalize_url(url));
    }

    fn prune(&mut self) {
        if self.seen.len() < 512 {
            return;
        }
        let window = self.window;
        let now = Instant::now();
        self.seen.retain(|_, at| now.duration_since(*at) < window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_copy_within_window_is_suppressed() {
        let mut c = SeenCache::new(Duration::from_secs(60));
        assert!(c.mark("https://youtu.be/abc"));
        assert!(!c.mark("https://youtu.be/abc"));
        // Same URL, different spelling (fragment/case) still dedupes.
        assert!(!c.mark("HTTPS://YOUTU.BE/abc#t=3"));
    }

    #[test]
    fn different_urls_both_pass() {
        let mut c = SeenCache::new(Duration::from_secs(60));
        assert!(c.mark("https://youtu.be/abc"));
        assert!(c.mark("https://youtu.be/def"));
    }

    #[test]
    fn zero_window_disables_suppression() {
        let mut c = SeenCache::new(Duration::ZERO);
        assert!(c.mark("https://youtu.be/abc"));
        assert!(c.mark("https://youtu.be/abc"));
    }

    #[test]
    fn forget_allows_immediate_retry() {
        let mut c = SeenCache::new(Duration::from_secs(60));
        assert!(c.mark("https://youtu.be/abc"));
        c.forget("https://youtu.be/abc");
        assert!(c.mark("https://youtu.be/abc"));
    }

    #[test]
    fn expired_entry_passes_again() {
        let mut c = SeenCache::new(Duration::from_millis(1));
        assert!(c.mark("https://youtu.be/abc"));
        std::thread::sleep(Duration::from_millis(5));
        assert!(c.mark("https://youtu.be/abc"));
    }
}
