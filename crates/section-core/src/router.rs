use opendal::Operator;
use std::collections::HashMap;
use std::str::FromStr;
use crate::{SectionConfig, SectionError, Result};
use crate::config::SourceConfig;

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
        let parsed = Self::parse_path(path)
            .ok_or_else(|| SectionError::InvalidPath(path.to_string()))?;

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
        let op = Operator::via_iter(
            opendal::Scheme::from_str(&source_cfg.provider)
                .map_err(|e| anyhow::anyhow!("unknown provider '{}': {e}", source_cfg.provider))?,
            source_cfg.options.iter().map(|(k, v)| (k.clone(), v.clone())),
        )
        .map_err(|e| anyhow::anyhow!("failed to build operator for '{}': {e}", source_cfg.provider))?;

        Ok(op)
    }
}
