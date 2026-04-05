use section_core::router::ParsedPath;
use section_core::{ContentCache, MetadataCache, Router, SectionConfig};
use std::collections::HashMap;
use std::time::Duration;

const DEFAULT_CONTENT_CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;

pub struct SectiondDataPlane {
    router: Router,
    metadata_caches: HashMap<String, MetadataCache>,
    content_caches: HashMap<String, ContentCache>,
}

impl SectiondDataPlane {
    pub fn new(config: &SectionConfig, router: Router) -> Self {
        let mut metadata_caches = HashMap::new();
        let mut content_caches = HashMap::new();

        for source in router.sources() {
            let cache_cfg = config.sources.get(&source).map(|cfg| &cfg.cache);
            let metadata_ttl_secs = cache_cfg.map(|cfg| cfg.metadata_ttl_secs).unwrap_or(60);
            let content_cache_enabled = cache_cfg
                .map(|cfg| cfg.content_ttl_secs > 0)
                .unwrap_or(true);

            if metadata_ttl_secs > 0 {
                metadata_caches.insert(
                    source.clone(),
                    MetadataCache::new(Duration::from_secs(metadata_ttl_secs)),
                );
            }
            if content_cache_enabled {
                content_caches.insert(
                    source.clone(),
                    ContentCache::new(DEFAULT_CONTENT_CACHE_MAX_BYTES),
                );
            }
        }

        Self {
            router,
            metadata_caches,
            content_caches,
        }
    }

    pub fn router(&self) -> &Router {
        &self.router
    }

    pub fn source_names(&self) -> Vec<String> {
        self.router.sources()
    }

    pub fn cached_stat(&mut self, parsed: &ParsedPath) -> Option<opendal::Metadata> {
        self.metadata_caches
            .get_mut(&parsed.source)?
            .get_stat(&parsed.sub_path)
            .cloned()
    }

    pub fn put_cached_stat(&mut self, parsed: &ParsedPath, meta: &opendal::Metadata) {
        if let Some(cache) = self.metadata_caches.get_mut(&parsed.source) {
            cache.put_stat(&parsed.sub_path, meta.clone());
        }
    }

    pub fn cached_listing(
        &mut self,
        parsed: &ParsedPath,
    ) -> Option<Vec<(String, opendal::Metadata)>> {
        self.metadata_caches
            .get_mut(&parsed.source)?
            .get_listing(&parsed.sub_path)
            .cloned()
    }

    pub fn put_cached_listing(
        &mut self,
        parsed: &ParsedPath,
        entries: &[(String, opendal::Metadata)],
    ) {
        if let Some(cache) = self.metadata_caches.get_mut(&parsed.source) {
            cache.put_listing(&parsed.sub_path, entries.to_vec());
        }
    }

    pub fn cached_content(&mut self, parsed: &ParsedPath) -> Option<Vec<u8>> {
        self.content_caches
            .get_mut(&parsed.source)?
            .get(&parsed.sub_path)
            .map(|data| data.to_vec())
    }

    pub fn put_cached_content(&mut self, parsed: &ParsedPath, data: &[u8]) {
        if let Some(cache) = self.content_caches.get_mut(&parsed.source) {
            cache.put(&parsed.sub_path, data.to_vec());
        }
    }

    pub fn invalidate_metadata_path(&mut self, parsed: &ParsedPath, recursive: bool) {
        if let Some(cache) = self.metadata_caches.get_mut(&parsed.source) {
            if recursive {
                cache.invalidate_prefix(&parsed.sub_path);
            }
            cache.invalidate(&parsed.sub_path);
        }
    }

    pub fn invalidate_all_path(&mut self, parsed: &ParsedPath, recursive: bool) {
        self.invalidate_metadata_path(parsed, recursive);
        if let Some(cache) = self.content_caches.get_mut(&parsed.source) {
            if recursive {
                cache.remove_prefix(&parsed.sub_path);
            } else {
                cache.remove(&parsed.sub_path);
            }
        }
    }

    pub fn clear_all_caches(&mut self) {
        for cache in self.metadata_caches.values_mut() {
            cache.clear();
        }
        for cache in self.content_caches.values_mut() {
            cache.clear();
        }
    }
}
