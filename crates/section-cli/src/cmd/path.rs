use crate::{PathAction, ResolveStrategyArg};
use anyhow::Result;
use sectiond::{PathResolveStrategy, SectiondControlPlane};
use serde_json::json;
use std::path::Path;

pub fn run(config_path: Option<&Path>, action: PathAction, json_mode: bool) -> Result<()> {
    let control_plane = SectiondControlPlane::load(config_path)?;

    match action {
        PathAction::Inspect { path } => {
            let snapshot = control_plane.path_inspect(&path)?;
            if json_mode {
                println!(
                    "{}",
                    serde_json::to_string(&json!({
                        "source_id": snapshot.source_id,
                        "local_root": snapshot.local_root,
                        "local_path": snapshot.local_path,
                        "source_path": snapshot.source_path,
                        "state": snapshot.state,
                        "detail": snapshot.detail,
                        "base_remote_version": snapshot.base_remote_version,
                        "current_remote_version": snapshot.current_remote_version,
                    }))?
                );
            } else {
                println!("Source: {}", snapshot.source_id);
                println!("Local root: {}", snapshot.local_root.display());
                println!("Local path: {}", snapshot.local_path.display());
                println!("Source path: {}", snapshot.source_path);
                println!("State: {}", snapshot.state);
                println!(
                    "Detail: local_present={}, dirty_local={}, dirty_remote={}, pinned={}, stale={}",
                    snapshot.detail.local_present,
                    snapshot.detail.dirty_local,
                    snapshot.detail.dirty_remote,
                    snapshot.detail.pinned,
                    snapshot.detail.stale,
                );
                println!(
                    "Versions: base_remote={:?}, current_remote={:?}",
                    snapshot.base_remote_version, snapshot.current_remote_version
                );
            }
        }
        PathAction::Compare { path } => {
            let snapshot = control_plane.path_compare(&path)?;
            if json_mode {
                println!("{}", serde_json::to_string(&snapshot)?);
            } else {
                println!("Source: {}", snapshot.source_id);
                println!("Local root: {}", snapshot.local_root.display());
                println!("Local path: {}", snapshot.local_path.display());
                println!("Source path: {}", snapshot.source_path);
                println!("State: {}", snapshot.state);
                println!(
                    "Presence: local={}, remote={}",
                    snapshot.local_present, snapshot.remote_present
                );
                println!(
                    "Versions: local={:?}, base_remote={:?}, current_remote={:?}",
                    snapshot.local_version,
                    snapshot.base_remote_version,
                    snapshot.current_remote_version
                );
                println!(
                    "Compare: local_matches_base={}, local_matches_current_remote={}, stale={}",
                    snapshot.local_matches_base,
                    snapshot.local_matches_current_remote,
                    snapshot.stale,
                );
            }
        }
        PathAction::Resolve { path, strategy } => {
            let strategy = match strategy {
                ResolveStrategyArg::UseLocal => PathResolveStrategy::UseLocal,
                ResolveStrategyArg::UseRemote => PathResolveStrategy::UseRemote,
            };
            let result = control_plane.path_resolve(&path, strategy)?;
            if json_mode {
                println!("{}", serde_json::to_string(&result)?);
            } else {
                println!(
                    "Resolved {} with {}. state={}, base_remote={:?}, current_remote={:?}",
                    result.source_path,
                    result.strategy,
                    result.state,
                    result.base_remote_version,
                    result.current_remote_version,
                );
            }
        }
    }

    Ok(())
}
