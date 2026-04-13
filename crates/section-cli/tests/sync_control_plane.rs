use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;

fn write_config(path: &Path, data_dir: &Path) {
    fs::write(
        path,
        format!(
            "data_dir = {:?}\nmount_point = \"/tmp/section-mount-test\"\n",
            data_dir.to_string_lossy()
        ),
    )
    .expect("write config");
}

fn run_section(config_path: &Path, args: &[&str]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_section");
    Command::new(bin)
        .arg("--config")
        .arg(config_path)
        .args(args)
        .output()
        .expect("run section")
}

#[test]
fn source_sync_compare_resolve_and_watch_work_together() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().join("data");
    let config_path = temp_dir.path().join("config.toml");
    let source_root = temp_dir.path().join("source-root");
    let local_root = temp_dir.path().join("bound-root");
    let local_file = local_root.join("docs").join("readme.txt");
    let remote_file = source_root.join("docs").join("readme.txt");

    fs::create_dir_all(remote_file.parent().expect("remote parent")).expect("create remote dir");
    fs::write(&remote_file, "remote-v1").expect("write remote seed");
    write_config(&config_path, &data_dir);

    let add = run_section(
        &config_path,
        &[
            "source",
            "add",
            "local",
            "--provider",
            "fs",
            "--opt",
            &format!("root={}", source_root.display()),
        ],
    );
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );

    let bind = run_section(
        &config_path,
        &[
            "source",
            "bind",
            "local",
            local_root.to_str().expect("utf8 local root"),
        ],
    );
    assert!(
        bind.status.success(),
        "{}",
        String::from_utf8_lossy(&bind.stderr)
    );

    let sync = run_section(&config_path, &["source", "sync", "local"]);
    assert!(
        sync.status.success(),
        "{}",
        String::from_utf8_lossy(&sync.stderr)
    );
    assert_eq!(
        fs::read_to_string(&local_file).expect("read local after sync"),
        "remote-v1"
    );

    let compare = run_section(
        &config_path,
        &[
            "--json",
            "path",
            "compare",
            local_file.to_str().expect("utf8 local file"),
        ],
    );
    assert!(
        compare.status.success(),
        "{}",
        String::from_utf8_lossy(&compare.stderr)
    );
    let compare: Value = serde_json::from_slice(&compare.stdout).expect("compare json");
    assert_eq!(compare["state"], "ready");
    assert_eq!(compare["local_matches_current_remote"], true);

    fs::write(&local_file, "local-v2").expect("overwrite local");
    fs::write(&remote_file, "remote-v2").expect("overwrite remote");

    let sync = run_section(&config_path, &["source", "sync", "local"]);
    assert!(
        sync.status.success(),
        "{}",
        String::from_utf8_lossy(&sync.stderr)
    );

    let compare = run_section(
        &config_path,
        &[
            "--json",
            "path",
            "compare",
            local_file.to_str().expect("utf8 local file"),
        ],
    );
    let compare: Value =
        serde_json::from_slice(&compare.stdout).expect("compare json after conflict");
    assert_eq!(compare["state"], "conflict");
    assert_eq!(compare["stale"], true);

    let watch = run_section(
        &config_path,
        &[
            "--json",
            "watch",
            local_root.to_str().expect("utf8 local root"),
            "--once",
        ],
    );
    assert!(
        watch.status.success(),
        "{}",
        String::from_utf8_lossy(&watch.stderr)
    );
    let watch_lines = String::from_utf8_lossy(&watch.stdout);
    assert!(watch_lines.contains("\"kind\":\"conflict_detected\""));
    assert!(watch_lines.contains("\"state\":\"syncing\""));

    let resolve = run_section(
        &config_path,
        &[
            "--json",
            "path",
            "resolve",
            local_file.to_str().expect("utf8 local file"),
            "--strategy",
            "use-local",
        ],
    );
    assert!(
        resolve.status.success(),
        "{}",
        String::from_utf8_lossy(&resolve.stderr)
    );
    let resolve: Value = serde_json::from_slice(&resolve.stdout).expect("resolve json");
    assert_eq!(resolve["state"], "ready");

    assert_eq!(
        fs::read_to_string(&remote_file).expect("read remote after resolve"),
        "local-v2"
    );
}

#[test]
fn source_sync_reports_cache_hits_on_second_noop_round() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().join("data");
    let config_path = temp_dir.path().join("config.toml");
    let source_root = temp_dir.path().join("source-root");
    let local_root = temp_dir.path().join("bound-root");
    let remote_file = source_root.join("docs").join("readme.txt");

    fs::create_dir_all(remote_file.parent().expect("remote parent")).expect("create remote dir");
    fs::write(&remote_file, "remote-v1").expect("write remote seed");
    write_config(&config_path, &data_dir);

    let add = run_section(
        &config_path,
        &[
            "source",
            "add",
            "local",
            "--provider",
            "fs",
            "--opt",
            &format!("root={}", source_root.display()),
        ],
    );
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );

    let bind = run_section(
        &config_path,
        &[
            "source",
            "bind",
            "local",
            local_root.to_str().expect("utf8 local root"),
        ],
    );
    assert!(
        bind.status.success(),
        "{}",
        String::from_utf8_lossy(&bind.stderr)
    );

    let first = run_section(&config_path, &["--json", "source", "sync", "local"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );

    let second = run_section(&config_path, &["--json", "source", "sync", "local"]);
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );

    let second: Value = serde_json::from_slice(&second.stdout).expect("second sync json");
    assert_eq!(second["local_scan"]["files"], 1);
    assert_eq!(second["local_scan"]["cache_hits"], 1);
    assert_eq!(second["local_scan"]["cache_misses"], 0);
    assert_eq!(second["remote_scan"]["body_fallbacks"], 0);
}

#[test]
fn source_sync_can_use_inventory_manifest_accelerator() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().join("data");
    let config_path = temp_dir.path().join("config.toml");
    let source_root = temp_dir.path().join("source-root");
    let local_root = temp_dir.path().join("bound-root");
    let remote_file = source_root.join("docs").join("readme.txt");
    let inventory_file = source_root.join("inventory.jsonl");

    fs::create_dir_all(remote_file.parent().expect("remote parent")).expect("create remote dir");
    fs::write(&remote_file, "remote-v1").expect("write remote seed");
    fs::write(
        &inventory_file,
        r#"{"path":"docs/readme.txt","kind":"file","version":"inventory-v1","size":9,"mtime_ms":1}"#,
    )
    .expect("write inventory");
    write_config(&config_path, &data_dir);

    let add = run_section(
        &config_path,
        &[
            "source",
            "add",
            "local",
            "--provider",
            "fs",
            "--opt",
            &format!("root={}", source_root.display()),
            "--opt",
            "section.sync_inventory_manifest=inventory.jsonl",
        ],
    );
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );

    let bind = run_section(
        &config_path,
        &[
            "source",
            "bind",
            "local",
            local_root.to_str().expect("utf8 local root"),
        ],
    );
    assert!(
        bind.status.success(),
        "{}",
        String::from_utf8_lossy(&bind.stderr)
    );

    let sync = run_section(&config_path, &["--json", "source", "sync", "local"]);
    assert!(
        sync.status.success(),
        "{}",
        String::from_utf8_lossy(&sync.stderr)
    );

    let sync: Value = serde_json::from_slice(&sync.stdout).expect("sync json");
    assert_eq!(
        sync["remote_scan"]["accelerator"],
        Value::String("inventory_manifest".to_string())
    );
    assert_eq!(sync["remote_scan"]["accelerated_entries"], 1);
}
