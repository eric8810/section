use anyhow::Result;
use sectiond::SectiondControlPlane;
use serde_json::json;
use std::path::Path;
use std::thread;
use std::time::Duration;

pub fn run(
    config_path: Option<&Path>,
    path: &Path,
    agentfs: bool,
    once: bool,
    interval_ms: u64,
    json_mode: bool,
) -> Result<()> {
    let control_plane = SectiondControlPlane::load(config_path)?;
    if agentfs {
        return run_agentfs_watch_loop(&control_plane, path, once, interval_ms, json_mode);
    }

    let mut after_id = 0_i64;

    loop {
        let events = control_plane.watch_path(path, after_id)?;
        for event in &events {
            after_id = event.id;
            if json_mode {
                println!("{}", serde_json::to_string(event)?);
            } else {
                println!(
                    "[{}] {} {} {}",
                    event.id, event.kind, event.path, event.state
                );
            }
        }

        if once {
            break;
        }

        thread::sleep(Duration::from_millis(interval_ms));
    }

    Ok(())
}

fn run_agentfs_watch_loop(
    control_plane: &SectiondControlPlane,
    path: &Path,
    once: bool,
    interval_ms: u64,
    json_mode: bool,
) -> Result<()> {
    let fs_ref = path.to_string_lossy().to_string();
    let mut after_seq = 0_i64;

    loop {
        let events = control_plane.watch_agentfs_events(&fs_ref, after_seq, 100)?;
        for event in &events {
            after_seq = event.seq;
            if json_mode {
                println!("{}", json!({ "stream": "agentfs", "event": event }));
            } else {
                println!(
                    "[agentfs:{}] {} {} {}",
                    event.seq, event.kind, event.fs_id, event.subject_id
                );
            }
        }

        if once {
            break;
        }

        thread::sleep(Duration::from_millis(interval_ms));
    }

    Ok(())
}
