use crate::PathAction;
use anyhow::Result;
use sectiond::SectiondControlPlane;
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
    }

    Ok(())
}
