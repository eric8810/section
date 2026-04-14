mod support;

use crate::support::{run_section, write_config};
use serde_json::Value;
use std::fs;

#[test]
fn source_bind_and_local_path_inspect_work_end_to_end() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().join("data");
    let config_path = temp_dir.path().join("config.toml");
    let source_root = temp_dir.path().join("source-root");
    let local_root = temp_dir.path().join("bound-root");
    let local_file = local_root.join("notes").join("todo.txt");

    fs::create_dir_all(&source_root).expect("create source root");
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
        "source add failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&add.stdout),
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
        "source bind failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&bind.stdout),
        String::from_utf8_lossy(&bind.stderr)
    );

    let marker = fs::read(local_root.join(".section").join("root.json")).expect("read marker");
    let marker: Value = serde_json::from_slice(&marker).expect("parse marker");
    assert_eq!(marker["source_id"], "local");

    fs::create_dir_all(local_file.parent().expect("parent")).expect("create local file parent");
    fs::write(&local_file, "hello").expect("write local file");

    let inspect = run_section(
        &config_path,
        &[
            "--json",
            "path",
            "inspect",
            local_file.to_str().expect("utf8 local file"),
        ],
    );
    assert!(
        inspect.status.success(),
        "path inspect failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&inspect.stdout),
        String::from_utf8_lossy(&inspect.stderr)
    );

    let inspect: Value = serde_json::from_slice(&inspect.stdout).expect("parse inspect json");
    assert_eq!(inspect["source_id"], "local");
    assert_eq!(
        inspect["local_root"].as_str(),
        Some(local_root.to_str().expect("utf8 local root"))
    );
    assert_eq!(
        inspect["local_path"].as_str(),
        Some(local_file.to_str().expect("utf8 local file"))
    );
    assert_eq!(inspect["source_path"], "notes/todo.txt");
    assert_eq!(inspect["state"], "ready");
    assert_eq!(inspect["detail"]["local_present"], true);
    assert_eq!(inspect["detail"]["dirty_local"], false);

    let source_list = run_section(&config_path, &["--json", "source", "list"]);
    assert!(
        source_list.status.success(),
        "source list failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&source_list.stdout),
        String::from_utf8_lossy(&source_list.stderr)
    );
    let source_list: Value =
        serde_json::from_slice(&source_list.stdout).expect("parse source list json");
    let sources = source_list.as_array().expect("source list array");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["name"], "local");
    assert_eq!(
        sources[0]["local_root"].as_str(),
        Some(local_root.to_str().expect("utf8 local root"))
    );
}
