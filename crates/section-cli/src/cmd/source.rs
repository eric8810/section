use crate::SourceAction;
use anyhow::Result;
use notify::{
    Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use sectiond::{
    SectiondControlPlane, SourceRegistryEntry, SourceSyncOptions, SourceSyncResult,
    SyncLifecycleEvent, SyncLifecycleObserver, SyncLifecycleStage,
};
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

/// Mask sensitive option values for display.
///
/// - Keys containing "secret", "password", "token", or equal to "private_key"
///   are fully masked as `***`.
/// - Keys containing "key_id" or "access_key" (but not "secret") are partially
///   masked: first 4 characters shown followed by `***`, or `***` if the value
///   is 4 characters or shorter.
/// - All other keys are shown in plain text.
fn mask_value(key: &str, value: &str) -> String {
    let lower = key.to_lowercase();
    if lower.contains("secret")
        || lower.contains("password")
        || lower.contains("token")
        || lower == "private_key"
    {
        return "***".to_string();
    }
    if lower.contains("key_id") || lower.contains("access_key") {
        if value.len() <= 4 {
            return "***".to_string();
        }
        return format!("{}***", &value[..4]);
    }
    value.to_string()
}

fn source_json(source: &SourceRegistryEntry) -> serde_json::Value {
    json!({
        "name": source.name,
        "provider": source.provider,
        "origin": source.origin,
        "metadata_ttl_secs": source.metadata_ttl_secs,
        "content_ttl_secs": source.content_ttl_secs,
        "local_root": source.local_root,
    })
}

pub fn run(config_path: Option<&Path>, action: SourceAction, json_mode: bool) -> Result<()> {
    let control_plane = SectiondControlPlane::load(config_path)?;

    match action {
        SourceAction::Add {
            name,
            provider,
            opt,
        } => {
            let options: HashMap<String, String> = opt.into_iter().collect();
            let entry = control_plane.source_add(&name, &provider, options)?;
            if json_mode {
                println!(
                    "{}",
                    json!({
                        "ok": true,
                        "message": format!("Source '{name}' added (provider: {provider})."),
                        "source": source_json(&entry),
                    })
                );
            } else {
                println!("Source '{name}' added (provider: {provider}).");
            }
        }
        SourceAction::Bind { name, local_root } => {
            let entry = control_plane.source_bind_local_root(&name, &local_root)?;
            if json_mode {
                println!(
                    "{}",
                    json!({
                        "ok": true,
                        "message": format!(
                            "Source '{name}' bound to local root {}.",
                            entry
                                .local_root
                                .as_ref()
                                .expect("bound source should have local root")
                                .display()
                        ),
                        "source": source_json(&entry),
                    })
                );
            } else {
                println!(
                    "Source '{name}' bound to local root {}.",
                    entry
                        .local_root
                        .as_ref()
                        .expect("bound source should have local root")
                        .display()
                );
            }
        }
        SourceAction::Unbind { name } => {
            control_plane.source_unbind_local_root(&name)?;
            if json_mode {
                println!(
                    "{}",
                    json!({
                        "ok": true,
                        "message": format!("Source '{name}' local root removed."),
                    })
                );
            } else {
                println!("Source '{name}' local root removed.");
            }
        }
        SourceAction::Sync {
            name,
            watch,
            concurrency,
            interval_secs,
        } => {
            let options = sync_options(concurrency);
            if watch {
                run_sync_watch_loop(&control_plane, &name, &options, interval_secs, json_mode)?;
            } else {
                run_sync_once(&control_plane, &name, &options, json_mode)?;
            }
        }
        SourceAction::Remove { name } => {
            control_plane.source_remove(&name)?;
            if json_mode {
                println!(
                    "{}",
                    json!({"ok": true, "message": format!("Source '{name}' removed.")})
                );
            } else {
                println!("Source '{name}' removed.");
            }
        }
        SourceAction::List => {
            let sources = control_plane.list_sources()?;
            if json_mode {
                let arr: Vec<serde_json::Value> = sources.iter().map(source_json).collect();
                println!("{}", serde_json::to_string(&arr)?);
            } else if sources.is_empty() {
                println!("No sources configured.");
            } else {
                for source in &sources {
                    println!(
                        "  {}  (provider: {}, origin: {}, metadata_ttl_secs: {}, content_ttl_secs: {}, local_root: {})",
                        source.name,
                        source.provider,
                        source.origin.as_str(),
                        source.metadata_ttl_secs,
                        source.content_ttl_secs,
                        source
                            .local_root
                            .as_ref()
                            .map(|path| path.display().to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    );
                    let mut keys: Vec<&String> = source.options.keys().collect();
                    keys.sort();
                    for key in keys {
                        let value = &source.options[key];
                        let display_value = mask_value(key, value);
                        println!("    {key} = {display_value}");
                    }
                }
            }
        }
    }
    Ok(())
}

fn sync_options(concurrency: usize) -> SourceSyncOptions {
    SourceSyncOptions {
        path_concurrency: concurrency.max(1),
        transfer_concurrency: concurrency.max(1),
        http_concurrency: concurrency.max(1),
        ..SourceSyncOptions::default()
    }
}

fn run_sync_once(
    control_plane: &SectiondControlPlane,
    name: &str,
    options: &SourceSyncOptions,
    json_mode: bool,
) -> Result<SourceSyncResult> {
    let lifecycle = (!json_mode).then(build_lifecycle_observer);
    let result = control_plane.source_sync_with_options(name, options, lifecycle)?;
    if json_mode {
        println!("{}", serde_json::to_string(&result)?);
    } else {
        print_sync_result(name, &result);
    }
    Ok(result)
}

fn run_sync_watch_loop(
    control_plane: &SectiondControlPlane,
    name: &str,
    options: &SourceSyncOptions,
    interval_secs: u64,
    json_mode: bool,
) -> Result<()> {
    run_sync_once(control_plane, name, options, json_mode)?;

    let local_root = control_plane.source_local_root(name)?;
    let poll_interval = Duration::from_secs(interval_secs.max(1));
    match start_notify_watcher(&local_root) {
        Ok((_watcher, rx)) => loop {
            match rx.recv_timeout(poll_interval) {
                Ok(Ok(event)) => {
                    if event_requires_sync(&event, &local_root) {
                        drain_notify_burst(&rx, &local_root);
                        run_sync_once(control_plane, name, options, json_mode)?;
                    }
                }
                Ok(Err(err)) => {
                    if !json_mode {
                        eprintln!(
                            "File watch error for '{}': {err}. Falling back to a sync run.",
                            local_root.display()
                        );
                    }
                    run_sync_once(control_plane, name, options, json_mode)?;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    run_sync_once(control_plane, name, options, json_mode)?;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if !json_mode {
                        eprintln!(
                            "File watch disconnected for '{}'. Falling back to polling.",
                            local_root.display()
                        );
                    }
                    return run_poll_watch_loop(
                        control_plane,
                        name,
                        options,
                        poll_interval,
                        json_mode,
                    );
                }
            }
        },
        Err(err) => {
            if !json_mode {
                eprintln!(
                    "File watch unavailable for '{}': {err}. Falling back to polling every {}s.",
                    local_root.display(),
                    interval_secs.max(1)
                );
            }
            run_poll_watch_loop(control_plane, name, options, poll_interval, json_mode)
        }
    }
}

fn run_poll_watch_loop(
    control_plane: &SectiondControlPlane,
    name: &str,
    options: &SourceSyncOptions,
    poll_interval: Duration,
    json_mode: bool,
) -> Result<()> {
    loop {
        std::thread::sleep(poll_interval);
        run_sync_once(control_plane, name, options, json_mode)?;
    }
}

fn start_notify_watcher(
    local_root: &Path,
) -> Result<(RecommendedWatcher, mpsc::Receiver<notify::Result<Event>>)> {
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })?;
    watcher.configure(NotifyConfig::default())?;
    watcher.watch(local_root, RecursiveMode::Recursive)?;
    Ok((watcher, rx))
}

fn drain_notify_burst(rx: &mpsc::Receiver<notify::Result<Event>>, local_root: &Path) {
    while let Ok(Ok(event)) = rx.recv_timeout(Duration::from_millis(200)) {
        if !event_requires_sync(&event, local_root) {
            continue;
        }
    }
}

fn event_requires_sync(event: &Event, local_root: &Path) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }

    let marker_dir = local_root.join(".section");
    event.paths.is_empty()
        || event
            .paths
            .iter()
            .any(|path| !path.starts_with(&marker_dir))
}

fn build_lifecycle_observer() -> SyncLifecycleObserver {
    let output_lock = Arc::new(Mutex::new(()));
    Arc::new(move |event: SyncLifecycleEvent| {
        let _guard = output_lock.lock().expect("stdout lock");
        match event.stage {
            SyncLifecycleStage::Progress => {
                let bytes_complete = event.bytes_complete.unwrap_or(0);
                let bytes_total = event.bytes_total.unwrap_or(0);
                if bytes_total > 0 {
                    println!(
                        "  [progress] {} {}/{} ({:.0}%)",
                        event.path,
                        human_bytes(bytes_complete),
                        human_bytes(bytes_total),
                        (bytes_complete as f64 / bytes_total as f64) * 100.0
                    );
                } else {
                    println!(
                        "  [progress] {} {}",
                        event.path,
                        human_bytes(bytes_complete)
                    );
                }
            }
            _ => {
                println!("  [{}] {}", lifecycle_stage_label(&event.stage), event.path);
            }
        }
    })
}

fn lifecycle_stage_label(stage: &SyncLifecycleStage) -> &'static str {
    match stage {
        SyncLifecycleStage::Queued => "queued",
        SyncLifecycleStage::Running => "running",
        SyncLifecycleStage::Progress => "progress",
        SyncLifecycleStage::Completed => "completed",
        SyncLifecycleStage::Failed => "failed",
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes}{}", UNITS[unit])
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

fn print_sync_result(name: &str, result: &SourceSyncResult) {
    let accelerator = result.remote_scan.accelerator.as_deref().unwrap_or("-");
    println!(
        "Source '{name}' synced. local_root={}, pulled={}, pushed={}, conflicts={}, events={}, local_cache_hits={}/{}, remote_metadata_hits={}, remote_stat_fallbacks={}, remote_body_fallbacks={}, remote_accelerator={}, accelerated_entries={}",
        result.local_root.display(),
        result.pulled,
        result.pushed,
        result.conflicts,
        result.events_emitted,
        result.local_scan.cache_hits,
        result.local_scan.files,
        result.remote_scan.metadata_hits,
        result.remote_scan.stat_fallbacks,
        result.remote_scan.body_fallbacks,
        accelerator,
        result.remote_scan.accelerated_entries,
    );
}
