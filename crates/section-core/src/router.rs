use crate::config::SourceConfig;
use crate::{Result, SectionConfig, SectionError};
use opendal::Operator;
use std::collections::HashMap;
use std::str::FromStr;

/// Routes paths like "{source}/{sub_path}" to the correct OpenDAL Operator.
pub struct Router {
    /// source_name -> Operator
    operators: HashMap<String, Operator>,
}

/// Parsed path components.
#[derive(Debug, Clone)]
pub struct ParsedPath {
    pub source: String,
    pub sub_path: String,
}

impl Router {
    /// Build a Router from config, creating an OpenDAL Operator for each source.
    pub fn from_config(config: &SectionConfig) -> Result<Self> {
        let mut operators = HashMap::new();

        for (source_name, source_cfg) in &config.sources {
            let op = Self::build_operator(source_cfg)?;
            operators.insert(source_name.clone(), op);
        }

        Ok(Self { operators })
    }

    /// Parse a section-relative path into (source, sub_path).
    ///
    /// Examples:
    ///   "my-s3/documents/report.pdf" -> ("my-s3", "documents/report.pdf")
    ///   "my-s3" -> ("my-s3", "")
    ///   "" -> returns None (root level, used for listing sources)
    pub fn parse_path(path: &str) -> Option<ParsedPath> {
        let path = path.trim_matches('/');
        if path.is_empty() {
            return None;
        }

        let (source, sub_path) = match path.split_once('/') {
            Some((s, p)) => (s.to_string(), p.to_string()),
            None => (path.to_string(), String::new()),
        };

        Some(ParsedPath { source, sub_path })
    }

    /// Get the Operator for a given source name.
    pub fn get_operator(&self, source: &str) -> Result<&Operator> {
        self.operators
            .get(source)
            .ok_or_else(|| SectionError::SourceNotFound(source.to_string()))
    }

    /// Resolve a full path to (Operator, sub_path).
    pub fn resolve(&self, path: &str) -> Result<(&Operator, String)> {
        let parsed =
            Self::parse_path(path).ok_or_else(|| SectionError::InvalidPath(path.to_string()))?;

        let op = self.get_operator(&parsed.source)?;
        Ok((op, parsed.sub_path))
    }

    /// List all registered source names.
    pub fn sources(&self) -> Vec<String> {
        let mut sources: Vec<String> = self.operators.keys().cloned().collect();
        sources.sort();
        sources
    }

    /// Add or replace an operator at runtime.
    pub fn add_operator(&mut self, source: &str, op: Operator) {
        self.operators.insert(source.to_string(), op);
    }

    fn build_operator(source_cfg: &SourceConfig) -> Result<Operator> {
        let mut options = source_cfg.options.clone();
        options.retain(|key, _| !key.starts_with("section."));
        if source_cfg.provider == "webdav" {
            if let Some(endpoint) = options.get_mut("endpoint") {
                *endpoint = endpoint.trim_end_matches('/').to_string();
            }
        }

        let op = Operator::via_iter(
            opendal::Scheme::from_str(&source_cfg.provider)
                .map_err(|e| anyhow::anyhow!("unknown provider '{}': {e}", source_cfg.provider))?,
            options.into_iter(),
        )
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to build operator for '{}': {e}",
                source_cfg.provider
            )
        })?;

        Ok(op)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CacheConfig;
    use std::path::PathBuf;

    fn fs_source(root: &str) -> SourceConfig {
        let mut options = HashMap::new();
        options.insert("root".to_string(), root.to_string());
        SourceConfig {
            provider: "fs".to_string(),
            options,
            cache: CacheConfig::default(),
        }
    }

    fn config_with_sources() -> SectionConfig {
        let mut sources = HashMap::new();
        sources.insert("b-src".to_string(), fs_source("/tmp/b"));
        sources.insert("a-src".to_string(), fs_source("/tmp/a"));

        SectionConfig {
            mount_point: PathBuf::from("/mnt/section"),
            data_dir: PathBuf::from("/tmp/section-data"),
            sources,
        }
    }

    #[test]
    fn parse_path_handles_root_source_and_nested_paths() {
        assert!(Router::parse_path("").is_none());
        assert_eq!(
            Router::parse_path("a-src").map(|path| (path.source, path.sub_path)),
            Some(("a-src".to_string(), "".to_string()))
        );
        assert_eq!(
            Router::parse_path("/a-src/docs/readme.md/").map(|path| (path.source, path.sub_path)),
            Some(("a-src".to_string(), "docs/readme.md".to_string()))
        );
    }

    #[test]
    fn sources_are_sorted() {
        let router = Router::from_config(&config_with_sources()).expect("router");
        assert_eq!(
            router.sources(),
            vec!["a-src".to_string(), "b-src".to_string()]
        );
    }

    #[test]
    fn resolve_rejects_unknown_or_empty_paths() {
        let router = Router::from_config(&config_with_sources()).expect("router");

        assert!(matches!(
            router.resolve(""),
            Err(SectionError::InvalidPath(path)) if path.is_empty()
        ));
        assert!(matches!(
            router.resolve("missing/file.txt"),
            Err(SectionError::SourceNotFound(source)) if source == "missing"
        ));
    }

    #[test]
    fn add_operator_replaces_existing_source() {
        let mut router = Router::from_config(&config_with_sources()).expect("router");
        let replacement = Operator::via_iter(
            opendal::Scheme::Fs,
            [("root".to_string(), "/tmp/replaced".to_string())],
        )
        .expect("replacement operator");

        router.add_operator("a-src", replacement);

        assert!(router.get_operator("a-src").is_ok());
    }

    #[test]
    fn reserved_section_options_are_not_forwarded_to_operator_builders() {
        let mut options = HashMap::new();
        options.insert("root".to_string(), "/tmp/a".to_string());
        options.insert(
            "section.sync_inventory_manifest".to_string(),
            "inventory.jsonl".to_string(),
        );
        let source = SourceConfig {
            provider: "fs".to_string(),
            options,
            cache: CacheConfig::default(),
        };

        let op = Router::build_operator(&source).expect("router with reserved section option");
        let meta = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(op.stat("/"))
            .expect("stat root");
        assert!(meta.is_dir());
    }
}
