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
            product_goal: "humans, agents, shell tools, and editors collaborate on the same mounted workspace".to_string(),
            primary_surface: "filesystem-first shared workspace".to_string(),
            control_plane: vec![
                "source registry management".to_string(),
                "status, health, and diagnostics".to_string(),
                "refresh and invalidation control".to_string(),
                "configuration and install/preflight flows".to_string(),
                "honest fallback workflows when mount is unavailable".to_string(),
            ],
            data_plane: vec![
                "mounted namespace exposure".to_string(),
                "path routing and operator access".to_string(),
                "permission and conflict semantics".to_string(),
                "cache-backed read/write/list behavior".to_string(),
                "shell/editor/script execution against the mounted tree".to_string(),
            ],
            ownership: vec![
                ResponsibilityBoundary {
                    responsibility: "source registry and operator lifecycle".to_string(),
                    owner: "sectiond".to_string(),
                },
                ResponsibilityBoundary {
                    responsibility: "routing, cache, refresh, permissions, diagnostics".to_string(),
                    owner: "sectiond".to_string(),
                },
                ResponsibilityBoundary {
                    responsibility: "source add/remove/list, status, refresh control".to_string(),
                    owner: "section-cli as a sectiond client".to_string(),
                },
                ResponsibilityBoundary {
                    responsibility: "mounted namespace exposure".to_string(),
                    owner: "platform mount adapters backed by sectiond".to_string(),
                },
            ],
            lifecycle: vec![
                "load runtime configuration".to_string(),
                "materialize a single merged source registry".to_string(),
                "expose control-plane and data-plane clients against one local state machine".to_string(),
                "surface health and shutdown semantics from the same runtime center".to_string(),
            ],
            transition_notes: vec![
                "the current repo is still pre-sectiond".to_string(),
                "config-file sources still coexist with the provider store during the transition".to_string(),
                "this crate is the first concrete boundary for the future daemon, not the final runtime".to_string(),
            ],
        }
    }
}
