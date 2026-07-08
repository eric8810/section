use crate::control_service::{
    ControlServiceRequest, ControlServiceResponse, ControlServiceRpcResult, ControlServiceStore,
};
use crate::{AgentFsError, AgentFsErrorPayload};
use anyhow::{Context, Result};
use axum::{extract::State, routing::post, Json, Router};
use section_core::SectionConfig;
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

type SharedStore = Arc<Mutex<ControlServiceStore>>;

pub async fn serve_control_service(
    config_path: Option<&Path>,
    addr: SocketAddr,
    json_mode: bool,
) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind Section Control Service on {addr}"))?;
    let local_addr = listener.local_addr()?;

    if json_mode {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "service": "section-control-service",
                "addr": local_addr.to_string(),
                "endpoint": format!("http://{local_addr}"),
            })
        );
    } else {
        println!("Section Control Service listening on http://{local_addr}");
    }
    std::io::stdout().flush()?;

    serve_control_service_listener(config_path, listener).await
}

pub async fn serve_control_service_listener(
    config_path: Option<&Path>,
    listener: tokio::net::TcpListener,
) -> Result<()> {
    let app = control_service_app(config_path)?;
    axum::serve(listener, app)
        .await
        .context("Section Control Service stopped unexpectedly")
}

pub fn control_service_app(config_path: Option<&Path>) -> Result<Router> {
    let config = SectionConfig::load(config_path)?;
    config.ensure_dirs()?;
    let store = Arc::new(Mutex::new(ControlServiceStore::open(&config)?));
    Ok(Router::new()
        .route("/v1/rpc", post(handle_rpc))
        .with_state(store))
}

async fn handle_rpc(
    State(store): State<SharedStore>,
    Json(request): Json<ControlServiceRequest>,
) -> Json<ControlServiceRpcResult> {
    let result = match store.lock() {
        Ok(store) => execute_rpc(&store, request),
        Err(err) => Err(anyhow::anyhow!("control service lock poisoned: {err}")),
    };
    Json(match result {
        Ok(response) => ControlServiceRpcResult::Ok { response },
        Err(err) => ControlServiceRpcResult::Err {
            error: error_payload(err),
        },
    })
}

fn execute_rpc(
    store: &ControlServiceStore,
    request: ControlServiceRequest,
) -> Result<ControlServiceResponse> {
    match request {
        ControlServiceRequest::LoginAgent {
            name,
            existing_installation_id,
            auth_token,
        } => Ok(ControlServiceResponse::AgentIdentity(
            store
                .login_agent(
                    &name,
                    existing_installation_id.as_deref(),
                    auth_token.as_deref(),
                )?
                .into(),
        )),
        ControlServiceRequest::SourceProfileByName { name, actor } => {
            let _actor = authenticate(store, actor)?;
            let (source_profile, source) = store.source_profile_by_name(&name)?;
            Ok(ControlServiceResponse::SourceProfile {
                source_profile,
                source,
            })
        }
        ControlServiceRequest::CreateFilesystem {
            name,
            source_profile_name,
            owner,
        } => {
            let owner = authenticate(store, owner)?;
            Ok(ControlServiceResponse::FilesystemCreate(
                store.create_filesystem(&name, &source_profile_name, &owner)?,
            ))
        }
        ControlServiceRequest::RollbackCreatedFilesystem { fs_id, actor } => {
            let actor = authenticate(store, actor)?;
            store.authorize_capability(
                &fs_id,
                &actor.agent_id,
                crate::AgentFsCapability::Manage,
            )?;
            store.rollback_created_filesystem(&fs_id)?;
            Ok(ControlServiceResponse::Unit)
        }
        ControlServiceRequest::ListFilesystemsForAgent { actor } => {
            let actor = authenticate(store, actor)?;
            Ok(ControlServiceResponse::Filesystems(
                store.list_filesystems_for_agent(&actor.agent_id)?,
            ))
        }
        ControlServiceRequest::ResolveFilesystem { fs_ref, actor } => {
            let actor = authenticate(store, actor)?;
            let resolved = store.resolve_filesystem(&fs_ref)?;
            store.authorize_capability(
                &resolved.fs.fs_id,
                &actor.agent_id,
                crate::AgentFsCapability::Read,
            )?;
            Ok(ControlServiceResponse::ResolvedFilesystem(resolved))
        }
        ControlServiceRequest::ListEvents { fs_id, actor } => {
            let actor = authenticate(store, actor)?;
            store.authorize_capability(&fs_id, &actor.agent_id, crate::AgentFsCapability::Read)?;
            Ok(ControlServiceResponse::Events(store.list_events(&fs_id)?))
        }
        ControlServiceRequest::ActiveRole { fs, actor } => {
            let actor = authenticate(store, actor)?;
            Ok(ControlServiceResponse::ActiveRole(
                store.active_role(&fs, &actor.agent_id)?,
            ))
        }
        ControlServiceRequest::AuthorizeCapability {
            fs_id,
            actor,
            capability,
        } => {
            let actor = authenticate(store, actor)?;
            Ok(ControlServiceResponse::Authorization(
                store.authorize_capability(&fs_id, &actor.agent_id, capability)?,
            ))
        }
        ControlServiceRequest::FsGrant {
            fs_id,
            actor,
            target_agent_id,
            role,
        } => {
            let actor = authenticate(store, actor)?;
            Ok(ControlServiceResponse::GrantMutation(store.fs_grant(
                &fs_id,
                &actor.agent_id,
                &target_agent_id,
                role,
            )?))
        }
        ControlServiceRequest::FsRevoke {
            fs_id,
            actor,
            target_agent_id,
        } => {
            let actor = authenticate(store, actor)?;
            Ok(ControlServiceResponse::RevokeMutation(store.fs_revoke(
                &fs_id,
                &actor.agent_id,
                &target_agent_id,
            )?))
        }
        ControlServiceRequest::FsShare {
            fs_id,
            actor,
            target_agent_id,
        } => {
            let actor = authenticate(store, actor)?;
            Ok(ControlServiceResponse::Share(store.fs_share(
                &fs_id,
                &actor.agent_id,
                &target_agent_id,
            )?))
        }
        ControlServiceRequest::AvailableShares { actor } => {
            let actor = authenticate(store, actor)?;
            Ok(ControlServiceResponse::AvailableShares(
                store.available_shares(&actor.agent_id)?,
            ))
        }
        ControlServiceRequest::AcceptShare { share_id, actor } => {
            let actor = authenticate(store, actor)?;
            let (accepted, source) =
                store.accept_share(&share_id, &actor.agent_id, &actor.installation_id)?;
            Ok(ControlServiceResponse::AcceptShare { accepted, source })
        }
        ControlServiceRequest::IssueCredential { fs_id, actor } => {
            let actor = authenticate(store, actor)?;
            Ok(ControlServiceResponse::IssuedCredential(
                store.issue_credential(&fs_id, &actor.agent_id, &actor.installation_id)?,
            ))
        }
    }
}

fn authenticate(
    store: &ControlServiceStore,
    actor: crate::control_service::AgentIdentityWire,
) -> Result<section_provider::AgentIdentityRecord> {
    let actor: section_provider::AgentIdentityRecord = actor.into();
    store.authenticate_agent(&actor)?;
    Ok(actor)
}

fn error_payload(err: anyhow::Error) -> AgentFsErrorPayload {
    err.downcast_ref::<AgentFsError>()
        .map(|agentfs| agentfs.payload().clone())
        .unwrap_or_else(|| {
            AgentFsError::new("operation_failed", err.to_string(), false)
                .with_details(serde_json::json!({ "command": "control_service" }))
                .payload()
                .clone()
        })
}
