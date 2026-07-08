pub mod agentfs;
pub mod contract;
pub mod control_plane;
pub mod control_service;
pub mod data_plane;
pub mod permission;
pub mod runtime;
pub mod sync;

pub use agentfs::{
    AgentFsAuthorization, AgentFsCapability, AgentFsCommitPathRecord, AgentFsCommitRecord,
    AgentFsCommitStagingRecord, AgentFsCredentialBindingRecord, AgentFsError, AgentFsErrorPayload,
    AgentFsEventRecord, AgentFsGrantRecord, AgentFsHeadRecord, AgentFsMaterializationState,
    AgentFsRecord, AgentFsRole, AgentFsShareRecord, AgentFsSourceProfileRecord,
};
pub use contract::{ResponsibilityBoundary, SectiondContract};
pub use control_plane::{
    PathDetailSnapshot, PathInspectSnapshot, RefreshResult, RootDiscoveryMarker,
    SectiondControlPlane, SourceRegistryEntry,
};
pub use control_service::{
    AgentFsAcceptResult, AgentFsAvailableShare, AgentFsShareResult, ControlServiceStore,
    IssuedAgentFsCredential,
};
pub use data_plane::SectiondDataPlane;
pub use permission::{ensure_directory_write_allowed, ensure_open_allowed};
pub use runtime::{
    SectiondRuntime, SourceOrigin, SourceSnapshot, SourceStatusSnapshot, StatusSnapshot,
};
pub use sync::{
    compare_path, list_watch_events, resolve_path, sync_source, sync_source_with_options,
    LocalScanStats, PathCompareSnapshot, PathResolveResult, PathResolveStrategy, RemoteScanStats,
    SourceSyncOptions, SourceSyncResult, SyncLifecycleEvent, SyncLifecycleObserver,
    SyncLifecycleStage,
};
