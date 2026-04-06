use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ResponsibilityBoundary {
    pub responsibility: String,
    pub owner: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SectiondContract {
    pub product_goal: String,
    pub primary_surface: String,
    pub control_plane: Vec<String>,
    pub data_plane: Vec<String>,
    pub ownership: Vec<ResponsibilityBoundary>,
    pub lifecycle: Vec<String>,
    pub transition_notes: Vec<String>,
}

impl Default for SectiondContract {
    fn default() -> Self {
        Self {
            product_goal: "humans and agents collaborate through truthful source/path sync into local bound directories".to_string(),
            primary_surface: "source/path sync with local-root bindings".to_string(),
            control_plane: vec![
                "source registry management".to_string(),
                "source local-root binding management".to_string(),
                "path inspect / compare / resolve".to_string(),
                "watch / event subscription".to_string(),
                "status, health, and diagnostics".to_string(),
            ],
            data_plane: vec![
                "bound local directory trees".to_string(),
                "local file and directory access".to_string(),
                "source/path state reconciliation".to_string(),
                "local/remote change ingestion".to_string(),
            ],
            ownership: vec![
                ResponsibilityBoundary {
                    responsibility: "source registry, local-root bindings, and path state".to_string(),
                    owner: "sectiond".to_string(),
                },
                ResponsibilityBoundary {
                    responsibility: "event emission, sync scheduling, and diagnostics".to_string(),
                    owner: "sectiond".to_string(),
                },
                ResponsibilityBoundary {
                    responsibility: "source/path control-plane commands".to_string(),
                    owner: "section-cli as a sectiond client".to_string(),
                },
            ],
            lifecycle: vec![
                "load runtime configuration".to_string(),
                "load source registry and local-root bindings".to_string(),
                "build one authoritative source/path state machine".to_string(),
                "serve control-plane clients against that shared state".to_string(),
                "emit source/path state changes".to_string(),
            ],
            transition_notes: vec![
                "the current repo is moving from helper-style commands toward sectiond-owned sync state".to_string(),
                "sync ingestion and eventing are only partially implemented today".to_string(),
                "this crate defines the active runtime boundary for the new route".to_string(),
            ],
        }
    }
}
