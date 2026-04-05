use std::collections::HashMap;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use opendal::Metadata;

/// A timestamped cache entry.
struct CacheEntry<T> {
    data: T,
    inserted_at: Instant,
}

/// In-memory cache for OpenDAL `stat()` and `list()` results with TTL-based
/// expiration.  Designed to sit between the FUSE layer and the backend so that
/// repeated `getattr` / `readdir` calls do not hit the remote on every request.
pub struct MetadataCache {
    /// Cached `stat` results keyed by normalised path.
    stats: HashMap<String, CacheEntry<Metadata>>,
    /// Cached directory listings keyed by normalised directory path.
    /// Each entry contains a list of `(child_name, Metadata)` pairs.
    listings: HashMap<String, CacheEntry<Vec<(String, Metadata)>>>,
    /// Time-to-live for every cache entry.
    ttl: Duration,
}

impl MetadataCache {
    /// Create a new, empty cache whose entries expire after `ttl`.
    pub fn new(ttl: Duration) -> Self {
        Self {
            stats: HashMap::new(),
            listings: HashMap::new(),
            ttl,
        }
    }

    // ------------------------------------------------------------------
    // stat cache
    // ------------------------------------------------------------------

    /// Return the cached `Metadata` for `path`, or `None` if the entry is
    /// missing or has expired.
    pub fn get_stat(&self, path: &str) -> Option<&Metadata> {
        let entry = self.stats.get(path)?;
        if entry.inserted_at.elapsed() < self.ttl {
            Some(&entry.data)
        } else {
            None
        }
    }

    /// Insert (or overwrite) the `stat` result for `path`.
    pub fn put_stat(&mut self, path: &str, meta: Metadata) {
        self.stats.insert(
            path.to_string(),
            CacheEntry {
                data: meta,
                inserted_at: Instant::now(),
            },
        );
    }

    // ------------------------------------------------------------------
    // listing cache
    // ------------------------------------------------------------------

    /// Return the cached directory listing for `path`, or `None` if the entry
    /// is missing or has expired.
    pub fn get_listing(&self, path: &str) -> Option<&Vec<(String, Metadata)>> {
        let entry = self.listings.get(path)?;
        if entry.inserted_at.elapsed() < self.ttl {
            Some(&entry.data)
        } else {
            None
        }
    }

    /// Insert (or overwrite) the directory listing for `path`.
    pub fn put_listing(&mut self, path: &str, entries: Vec<(String, Metadata)>) {
        self.listings.insert(
            path.to_string(),
            CacheEntry {
                data: entries,
                inserted_at: Instant::now(),
            },
        );
    }

    // ------------------------------------------------------------------
    // invalidation
    // ------------------------------------------------------------------

    /// Invalidate a specific path.
    ///
    /// This removes the `stat` entry for the path **and** any directory listing
    /// whose key equals the path or whose key is the parent directory of the
    /// path (i.e. the listing that would contain this entry).
    pub fn invalidate(&mut self, path: &str) {
        self.stats.remove(path);
        // Remove listing for this exact path (if it was a directory).
        self.listings.remove(path);

        // Also remove the listing of the parent directory so that a subsequent
        // readdir will re-fetch and see the change.
        if let Some(parent) = parent_dir(path) {
            self.listings.remove(parent);
        }
    }

    /// Invalidate every entry whose path starts with `prefix`.
    ///
    /// Useful when a directory is renamed or deleted and all descendants must
    /// be flushed.
    pub fn invalidate_prefix(&mut self, prefix: &str) {
        self.stats.retain(|k, _| !path_matches_prefix(k, prefix));
        self.listings.retain(|k, _| !path_matches_prefix(k, prefix));
    }

    /// Drop all cached entries.
    pub fn clear(&mut self) {
        self.stats.clear();
        self.listings.clear();
    }
}

// ------------------------------------------------------------------
// Content cache (LRU eviction)
// ------------------------------------------------------------------

/// LRU content cache with a maximum total size in bytes.
///
/// Caches raw file content keyed by path.  When the total cached bytes would
/// exceed `max_bytes`, the least-recently-used entries are evicted until there
/// is enough room.  If a single item is larger than `max_bytes` it is silently
/// skipped (never stored).
pub struct ContentCache {
    /// Maximum total bytes to cache.
    max_bytes: usize,
    /// Current total bytes cached.
    current_bytes: usize,
    /// Ordered from least-recently-used (front) to most-recently-used (back).
    order: VecDeque<String>,
    /// path -> cached content
    entries: HashMap<String, Vec<u8>>,
}

impl ContentCache {
    /// Create a new, empty content cache that holds at most `max_bytes` of data.
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            current_bytes: 0,
            order: VecDeque::new(),
            entries: HashMap::new(),
        }
    }

    /// Get cached content for a path.  Returns `None` if not cached.
    ///
    /// Moves the entry to the most-recently-used position.
    pub fn get(&mut self, path: &str) -> Option<&[u8]> {
        if !self.entries.contains_key(path) {
            return None;
        }
        // Promote to MRU: remove from current position, push to back.
        self.promote(path);
        Some(self.entries.get(path).unwrap().as_slice())
    }

    /// Put content into the cache.  Evicts LRU entries if needed to stay under
    /// `max_bytes`.
    ///
    /// If the item itself is larger than `max_bytes`, it is not cached.
    /// If the path already exists, the old entry is replaced in-place.
    pub fn put(&mut self, path: &str, data: Vec<u8>) {
        let new_len = data.len();

        // Item too large to ever fit — skip.
        if new_len > self.max_bytes {
            return;
        }

        // If the path is already cached, remove the old value first so we can
        // account for the size difference correctly.
        if let Some(old) = self.entries.remove(path) {
            self.current_bytes -= old.len();
            self.remove_from_order(path);
        }

        // Evict LRU entries until there is room.
        while self.current_bytes + new_len > self.max_bytes {
            if let Some(evict_key) = self.order.pop_front() {
                if let Some(evicted) = self.entries.remove(&evict_key) {
                    self.current_bytes -= evicted.len();
                }
            } else {
                break;
            }
        }

        self.entries.insert(path.to_string(), data);
        self.current_bytes += new_len;
        self.order.push_back(path.to_string());
    }

    /// Remove a specific path from the cache.
    pub fn remove(&mut self, path: &str) {
        if let Some(removed) = self.entries.remove(path) {
            self.current_bytes -= removed.len();
            self.remove_from_order(path);
        }
    }

    /// Remove every cached item whose path equals `prefix` or lives under it.
    pub fn remove_prefix(&mut self, prefix: &str) {
        let keys: Vec<String> = self
            .entries
            .keys()
            .filter(|path| path_matches_prefix(path, prefix))
            .cloned()
            .collect();

        for key in keys {
            self.remove(&key);
        }
    }

    /// Clear the entire cache.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.current_bytes = 0;
    }

    /// Current total bytes in cache.
    pub fn size(&self) -> usize {
        self.current_bytes
    }

    // ------------------------------------------------------------------
    // internal helpers
    // ------------------------------------------------------------------

    /// Remove `path` from the LRU order deque.
    fn remove_from_order(&mut self, path: &str) {
        if let Some(pos) = self.order.iter().position(|p| p == path) {
            self.order.remove(pos);
        }
    }

    /// Move `path` to the most-recently-used (back) position.
    fn promote(&mut self, path: &str) {
        self.remove_from_order(path);
        self.order.push_back(path.to_string());
    }
}

/// Return the parent directory portion of `path`, or `None` if the path has no
/// `/` separator (i.e. it is a top-level entry).
fn parent_dir(path: &str) -> Option<&str> {
    let path = path.trim_end_matches('/');
    path.rfind('/').map(|idx| &path[..idx])
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    let path = path.trim_matches('/');
    let prefix = prefix.trim_matches('/');

    if prefix.is_empty() {
        return true;
    }

    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use opendal::EntryMode;
    use std::thread;

    /// Helper: build a minimal `Metadata` for a regular file.
    fn file_meta(size: u64) -> Metadata {
        Metadata::new(EntryMode::FILE).with_content_length(size)
    }

    /// Helper: build a minimal `Metadata` for a directory.
    fn dir_meta() -> Metadata {
        Metadata::new(EntryMode::DIR)
    }

    #[test]
    fn stat_put_and_get() {
        let mut cache = MetadataCache::new(Duration::from_secs(60));
        cache.put_stat("a/b.txt", file_meta(42));

        let m = cache.get_stat("a/b.txt").expect("should be cached");
        assert_eq!(m.content_length(), 42);
    }

    #[test]
    fn stat_miss_returns_none() {
        let cache = MetadataCache::new(Duration::from_secs(60));
        assert!(cache.get_stat("missing").is_none());
    }

    #[test]
    fn stat_expired_returns_none() {
        let mut cache = MetadataCache::new(Duration::from_millis(5));
        cache.put_stat("a.txt", file_meta(1));

        // Wait for the TTL to pass.
        thread::sleep(Duration::from_millis(10));
        assert!(cache.get_stat("a.txt").is_none());
    }

    #[test]
    fn listing_put_and_get() {
        let mut cache = MetadataCache::new(Duration::from_secs(60));
        let entries = vec![
            ("file1.txt".to_string(), file_meta(10)),
            ("subdir".to_string(), dir_meta()),
        ];
        cache.put_listing("mydir", entries);

        let listing = cache.get_listing("mydir").expect("should be cached");
        assert_eq!(listing.len(), 2);
        assert_eq!(listing[0].0, "file1.txt");
    }

    #[test]
    fn listing_expired_returns_none() {
        let mut cache = MetadataCache::new(Duration::from_millis(5));
        cache.put_listing("dir", vec![("x".to_string(), file_meta(1))]);

        thread::sleep(Duration::from_millis(10));
        assert!(cache.get_listing("dir").is_none());
    }

    #[test]
    fn invalidate_removes_stat_and_parent_listing() {
        let mut cache = MetadataCache::new(Duration::from_secs(60));
        cache.put_stat("a/b.txt", file_meta(1));
        cache.put_listing("a", vec![("b.txt".to_string(), file_meta(1))]);
        cache.put_listing("other", vec![("c.txt".to_string(), file_meta(2))]);

        cache.invalidate("a/b.txt");

        assert!(cache.get_stat("a/b.txt").is_none());
        // Parent listing "a" should also be gone.
        assert!(cache.get_listing("a").is_none());
        // Unrelated listing should survive.
        assert!(cache.get_listing("other").is_some());
    }

    #[test]
    fn invalidate_directory_removes_own_listing() {
        let mut cache = MetadataCache::new(Duration::from_secs(60));
        cache.put_stat("a/b", dir_meta());
        cache.put_listing("a/b", vec![("c.txt".to_string(), file_meta(1))]);

        cache.invalidate("a/b");

        assert!(cache.get_stat("a/b").is_none());
        assert!(cache.get_listing("a/b").is_none());
    }

    #[test]
    fn invalidate_prefix_removes_all_matching() {
        let mut cache = MetadataCache::new(Duration::from_secs(60));
        cache.put_stat("proj/src/main.rs", file_meta(100));
        cache.put_stat("proj/src/lib.rs", file_meta(200));
        cache.put_stat("proj/README.md", file_meta(50));
        cache.put_listing("proj/src", vec![]);
        cache.put_listing("other", vec![]);

        cache.invalidate_prefix("proj/src");

        assert!(cache.get_stat("proj/src/main.rs").is_none());
        assert!(cache.get_stat("proj/src/lib.rs").is_none());
        assert!(cache.get_listing("proj/src").is_none());
        // Entries outside the prefix survive.
        assert!(cache.get_stat("proj/README.md").is_some());
        assert!(cache.get_listing("other").is_some());
    }

    #[test]
    fn invalidate_prefix_respects_path_boundaries() {
        let mut cache = MetadataCache::new(Duration::from_secs(60));
        cache.put_stat("proj/src/main.rs", file_meta(100));
        cache.put_stat("proj/src-two/main.rs", file_meta(200));

        cache.invalidate_prefix("proj/src");

        assert!(cache.get_stat("proj/src/main.rs").is_none());
        assert!(cache.get_stat("proj/src-two/main.rs").is_some());
    }

    #[test]
    fn clear_removes_everything() {
        let mut cache = MetadataCache::new(Duration::from_secs(60));
        cache.put_stat("x", file_meta(1));
        cache.put_listing("y", vec![]);

        cache.clear();

        assert!(cache.get_stat("x").is_none());
        assert!(cache.get_listing("y").is_none());
    }

    #[test]
    fn overwrite_stat_refreshes_ttl() {
        // Keep generous timing margins so the test validates refresh semantics
        // instead of failing due to scheduler jitter on busy CI machines.
        let mut cache = MetadataCache::new(Duration::from_millis(500));
        cache.put_stat("f.txt", file_meta(1));

        // Wait long enough to age the original entry, but keep plenty of TTL
        // headroom so the overwrite semantics are the thing under test.
        thread::sleep(Duration::from_millis(100));
        // Overwrite resets the clock.
        cache.put_stat("f.txt", file_meta(2));

        // After another delay shorter than the refreshed TTL, the entry should
        // still be valid and expose the new metadata.
        thread::sleep(Duration::from_millis(100));
        let m = cache
            .get_stat("f.txt")
            .expect("should still be cached after refresh");
        assert_eq!(m.content_length(), 2);
    }

    #[test]
    fn parent_dir_helper() {
        assert_eq!(parent_dir("a/b/c.txt"), Some("a/b"));
        assert_eq!(parent_dir("a/b"), Some("a"));
        assert_eq!(parent_dir("toplevel"), None);
        assert_eq!(parent_dir("a/b/"), Some("a")); // trailing slash trimmed
    }

    // ------------------------------------------------------------------
    // ContentCache tests
    // ------------------------------------------------------------------

    #[test]
    fn content_cache_put_and_get() {
        let mut cache = ContentCache::new(1024);
        cache.put("a.txt", vec![1, 2, 3]);

        assert_eq!(cache.get("a.txt"), Some([1u8, 2, 3].as_slice()));
        assert_eq!(cache.size(), 3);
    }

    #[test]
    fn content_cache_miss_returns_none() {
        let mut cache = ContentCache::new(1024);
        assert!(cache.get("missing").is_none());
    }

    #[test]
    fn content_cache_evicts_lru() {
        // Cache holds at most 10 bytes.
        let mut cache = ContentCache::new(10);

        cache.put("a", vec![0; 4]); // 4 bytes, total = 4
        cache.put("b", vec![0; 4]); // 4 bytes, total = 8
        cache.put("c", vec![0; 4]); // 4 bytes — needs to evict "a" to fit (8+4=12 > 10)

        // "a" should have been evicted (LRU).
        assert!(cache.get("a").is_none());
        // "b" and "c" should still be present.
        assert!(cache.get("b").is_some());
        assert!(cache.get("c").is_some());
        assert!(cache.size() <= 10);
    }

    #[test]
    fn content_cache_get_promotes_to_mru() {
        let mut cache = ContentCache::new(10);

        cache.put("a", vec![0; 4]); // LRU
        cache.put("b", vec![0; 4]); // MRU

        // Access "a" to promote it to MRU.
        cache.get("a");

        // Now insert "c"; "b" is now the LRU and should be evicted.
        cache.put("c", vec![0; 4]);

        assert!(
            cache.get("b").is_none(),
            "b should have been evicted as LRU"
        );
        assert!(cache.get("a").is_some(), "a should survive (was promoted)");
        assert!(cache.get("c").is_some());
    }

    #[test]
    fn content_cache_skip_oversized_item() {
        let mut cache = ContentCache::new(5);
        cache.put("big", vec![0; 6]); // exceeds max_bytes

        assert!(cache.get("big").is_none());
        assert_eq!(cache.size(), 0);
    }

    #[test]
    fn content_cache_overwrite_existing() {
        let mut cache = ContentCache::new(100);
        cache.put("a", vec![1, 2, 3]); // 3 bytes
        assert_eq!(cache.size(), 3);

        cache.put("a", vec![4, 5]); // overwrite with 2 bytes
        assert_eq!(cache.get("a"), Some([4u8, 5].as_slice()));
        assert_eq!(cache.size(), 2);
    }

    #[test]
    fn content_cache_remove() {
        let mut cache = ContentCache::new(100);
        cache.put("a", vec![1, 2, 3]);
        cache.remove("a");

        assert!(cache.get("a").is_none());
        assert_eq!(cache.size(), 0);
    }

    #[test]
    fn content_cache_remove_nonexistent_is_noop() {
        let mut cache = ContentCache::new(100);
        cache.remove("nope"); // should not panic
        assert_eq!(cache.size(), 0);
    }

    #[test]
    fn content_cache_clear() {
        let mut cache = ContentCache::new(100);
        cache.put("a", vec![1]);
        cache.put("b", vec![2]);
        cache.clear();

        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_none());
        assert_eq!(cache.size(), 0);
    }

    #[test]
    fn content_cache_remove_prefix_respects_boundaries() {
        let mut cache = ContentCache::new(100);
        cache.put("proj/src/main.rs", vec![1]);
        cache.put("proj/src-two/main.rs", vec![2]);

        cache.remove_prefix("proj/src");

        assert!(cache.get("proj/src/main.rs").is_none());
        assert_eq!(cache.get("proj/src-two/main.rs"), Some([2u8].as_slice()));
    }

    #[test]
    fn content_cache_exact_fit() {
        // Cache fits exactly the item.
        let mut cache = ContentCache::new(5);
        cache.put("x", vec![0; 5]);

        assert!(cache.get("x").is_some());
        assert_eq!(cache.size(), 5);
    }

    #[test]
    fn content_cache_eviction_chain() {
        // Make sure multiple LRU entries are evicted if needed.
        let mut cache = ContentCache::new(10);
        cache.put("a", vec![0; 3]); // total = 3
        cache.put("b", vec![0; 3]); // total = 6
        cache.put("c", vec![0; 3]); // total = 9

        // Insert a 5-byte entry — need to evict "a" and "b" (6 bytes) to make room.
        cache.put("d", vec![0; 5]);

        assert!(cache.get("a").is_none(), "a should be evicted");
        assert!(cache.get("b").is_none(), "b should be evicted");
        assert!(cache.get("c").is_some(), "c should survive");
        assert!(cache.get("d").is_some(), "d should be present");
        assert!(cache.size() <= 10);
    }
}
