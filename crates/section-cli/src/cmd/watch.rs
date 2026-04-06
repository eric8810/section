use anyhow::Result;
use sectiond::SectiondControlPlane;
use std::path::Path;
use std::thread;
use std::time::Duration;

pub fn run(
    config_path: Option<&Path>,
    path: &Path,
    once: bool,
    interval_ms: u64,
    json_mode: bool,
) -> Result<()> {
    let control_plane = SectiondControlPlane::load(config_path)?;
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
