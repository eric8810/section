mod support;

use crate::support::{assert_success, run_section, write_config};
use serde_json::Value;
use std::fs;

#[test]
fn agent_register_and_fs_create_write_shared_metadata() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().join("owner-data");
    let config_path = temp_dir.path().join("owner.toml");
    let remote_root = temp_dir.path().join("remote");

    fs::create_dir_all(&remote_root).expect("create remote root");
    write_config(&config_path, &data_dir);

    let register = run_section(&config_path, &["--json", "agent", "register", "agent-a"]);
    assert_success(&register, "agent register");
    let register: Value = serde_json::from_slice(&register.stdout).expect("register json");
    let owner_id = register["agent"]["agent_id"]
        .as_str()
        .expect("owner id")
        .to_string();
    assert!(owner_id.starts_with("agt_"));

    let create = run_section(
        &config_path,
        &[
            "--json",
            "fs",
            "create",
            "project",
            "--provider",
            "fs",
            "--opt",
            &format!("root={}", remote_root.display()),
        ],
    );
    assert_success(&create, "fs create");
    let create: Value = serde_json::from_slice(&create.stdout).expect("create json");
    let fs_id = create["fs"]["fs_id"].as_str().expect("fs id").to_string();
    assert!(fs_id.starts_with("fs_"));
    assert_eq!(create["fs"]["owner_agent_id"], owner_id);
    assert_eq!(create["fs"]["source_name"], "project");

    let fs_json = fs::read(remote_root.join(".section/agentfs/fs.json")).expect("read fs.json");
    let fs_json: Value = serde_json::from_slice(&fs_json).expect("parse fs.json");
    assert_eq!(fs_json["fs_id"], fs_id);
    assert_eq!(fs_json["owner_agent_id"], owner_id);

    let head_json =
        fs::read(remote_root.join(".section/agentfs/heads/current.json")).expect("read head");
    let head_json: Value = serde_json::from_slice(&head_json).expect("parse head");
    assert_eq!(head_json["fs_id"], fs_id);
    assert!(head_json["commit_id"].is_null());

    let grant_count = fs::read_dir(remote_root.join(".section/agentfs/grants"))
        .expect("grants dir")
        .count();
    assert_eq!(grant_count, 1);
    let events = fs::read_dir(remote_root.join(".section/agentfs/events"))
        .expect("events dir")
        .map(|entry| {
            let entry = entry.expect("event entry");
            let event: Value = serde_json::from_slice(&fs::read(entry.path()).expect("read event"))
                .expect("event json");
            event["kind"].as_str().expect("kind").to_string()
        })
        .collect::<Vec<_>>();
    assert!(events.contains(&"fs.created".to_string()));

    let list = run_section(&config_path, &["--json", "fs", "list"]);
    assert_success(&list, "fs list");
    let list: Value = serde_json::from_slice(&list.stdout).expect("list json");
    let filesystems = list.as_array().expect("fs list array");
    assert_eq!(filesystems.len(), 1);
    assert_eq!(filesystems[0]["fs_id"], fs_id);

    let status = run_section(&config_path, &["--json", "fs", "status", "project"]);
    assert_success(&status, "fs status");
    let status: Value = serde_json::from_slice(&status.stdout).expect("status json");
    assert_eq!(status["status"]["role"], "owner");
    assert_eq!(status["status"]["head"]["commit_id"], Value::Null);
}

#[test]
fn fs_create_rejects_existing_source_without_overwriting_it() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().join("owner-data");
    let config_path = temp_dir.path().join("owner.toml");
    let original_root = temp_dir.path().join("original-remote");
    let attempted_root = temp_dir.path().join("attempted-remote");

    fs::create_dir_all(&original_root).expect("create original remote root");
    fs::create_dir_all(&attempted_root).expect("create attempted remote root");
    write_config(&config_path, &data_dir);

    let register = run_section(&config_path, &["--json", "agent", "register", "owner"]);
    assert_success(&register, "agent register");

    let source_add = run_section(
        &config_path,
        &[
            "source",
            "add",
            "project",
            "--provider",
            "fs",
            "--opt",
            &format!("root={}", original_root.display()),
        ],
    );
    assert_success(&source_add, "source add");

    let create = run_section(
        &config_path,
        &[
            "--json",
            "fs",
            "create",
            "project",
            "--provider",
            "fs",
            "--opt",
            &format!("root={}", attempted_root.display()),
        ],
    );
    assert!(
        !create.status.success(),
        "fs create unexpectedly overwrote an existing source\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&create.stdout),
        String::from_utf8_lossy(&create.stderr)
    );

    let source_list = run_section(&config_path, &["--json", "source", "list"]);
    assert_success(&source_list, "source list");
    let source_list: Value = serde_json::from_slice(&source_list.stdout).expect("source list json");
    let sources = source_list.as_array().expect("source list array");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["name"], "project");
    assert_eq!(
        sources[0]["options"]["root"].as_str(),
        Some(original_root.to_str().expect("utf8 original root"))
    );
    assert!(
        !attempted_root.join(".section/agentfs/fs.json").exists(),
        "failed fs create must not initialize attempted remote metadata"
    );
}

#[test]
fn granted_writer_can_attach_without_syncing_agentfs_metadata_as_content() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let remote_root = temp_dir.path().join("remote");
    let owner_data = temp_dir.path().join("owner-data");
    let writer_data = temp_dir.path().join("writer-data");
    let owner_config = temp_dir.path().join("owner.toml");
    let writer_config = temp_dir.path().join("writer.toml");
    let owner_root = temp_dir.path().join("owner-root");
    let writer_root = temp_dir.path().join("writer-root");
    let writer_second_root = temp_dir.path().join("writer-second-root");

    fs::create_dir_all(&remote_root).expect("create remote root");
    write_config(&owner_config, &owner_data);
    write_config(&writer_config, &writer_data);

    let owner_register = run_section(&owner_config, &["--json", "agent", "register", "owner"]);
    assert_success(&owner_register, "owner register");

    let create = run_section(
        &owner_config,
        &[
            "--json",
            "fs",
            "create",
            "project",
            "--provider",
            "fs",
            "--opt",
            &format!("root={}", remote_root.display()),
        ],
    );
    assert_success(&create, "owner fs create");
    let create: Value = serde_json::from_slice(&create.stdout).expect("create json");
    let fs_id = create["fs"]["fs_id"].as_str().expect("fs id").to_string();

    let writer_register = run_section(&writer_config, &["--json", "agent", "register", "writer"]);
    assert_success(&writer_register, "writer register");
    let writer_register: Value =
        serde_json::from_slice(&writer_register.stdout).expect("writer register json");
    let writer_id = writer_register["agent"]["agent_id"]
        .as_str()
        .expect("writer id")
        .to_string();

    let writer_source = run_section(
        &writer_config,
        &[
            "source",
            "add",
            "project",
            "--provider",
            "fs",
            "--opt",
            &format!("root={}", remote_root.display()),
        ],
    );
    assert_success(&writer_source, "writer backing source add");

    let grant = run_section(
        &owner_config,
        &[
            "--json", "fs", "grant", "project", &writer_id, "--role", "writer",
        ],
    );
    assert_success(&grant, "grant writer");
    let grant: Value = serde_json::from_slice(&grant.stdout).expect("grant json");
    assert_eq!(grant["grant"]["agent_id"], writer_id);
    assert_eq!(grant["grant"]["role"], "writer");

    let attach = run_section(
        &writer_config,
        &[
            "--json",
            "fs",
            "attach",
            "project",
            writer_root.to_str().expect("utf8 writer root"),
        ],
    );
    assert_success(&attach, "writer attach");
    let attach: Value = serde_json::from_slice(&attach.stdout).expect("attach json");
    assert_eq!(attach["attach"]["fs"]["fs_id"], fs_id);
    assert_eq!(
        attach["attach"]["local_root"].as_str(),
        Some(writer_root.to_str().expect("utf8 writer root"))
    );

    let marker = fs::read(writer_root.join(".section/root.json")).expect("read writer root marker");
    let marker: Value = serde_json::from_slice(&marker).expect("marker json");
    assert_eq!(marker["schema_version"], 1);
    assert_eq!(marker["fs_id"], fs_id);
    assert_eq!(marker["source_id"], "project");
    assert_eq!(marker["agent_id"], writer_id);
    assert!(marker["base_commit_id"].is_null());

    assert!(
        !writer_root.join(".section/agentfs").exists(),
        "AgentFS shared metadata must not sync into the working copy as content"
    );

    let status = run_section(&writer_config, &["--json", "fs", "status", "project"]);
    assert_success(&status, "writer fs status");
    let status: Value = serde_json::from_slice(&status.stdout).expect("status json");
    assert_eq!(status["status"]["role"], "writer");
    assert_eq!(status["status"]["base_commit_id"], Value::Null);

    let writer_file = writer_root.join("docs").join("note.txt");
    fs::create_dir_all(writer_file.parent().expect("writer file parent"))
        .expect("create writer docs");
    fs::write(&writer_file, "hello from writer").expect("write writer file");

    let commit_status = run_section(
        &writer_config,
        &[
            "--json",
            "commit",
            "status",
            writer_root.to_str().expect("utf8 writer root"),
        ],
    );
    assert_success(&commit_status, "commit status");
    let commit_status: Value =
        serde_json::from_slice(&commit_status.stdout).expect("commit status json");
    assert_eq!(commit_status["status"]["stale"], false);
    let dirty_paths = commit_status["status"]["dirty_paths"]
        .as_array()
        .expect("dirty paths");
    assert!(
        dirty_paths
            .iter()
            .any(|path| path["path"] == "docs/note.txt" && path["op"] == "create"),
        "dirty paths should include created writer file: {dirty_paths:?}"
    );

    let commit = run_section(
        &writer_config,
        &[
            "--json",
            "commit",
            "apply",
            writer_root.to_str().expect("utf8 writer root"),
            "--message",
            "add writer note",
        ],
    );
    assert_success(&commit, "commit apply");
    let commit: Value = serde_json::from_slice(&commit.stdout).expect("commit json");
    let commit_id = commit["commit"]["commit_id"]
        .as_str()
        .expect("commit id")
        .to_string();
    assert!(commit_id.starts_with("cmt_"));
    assert_eq!(commit["commit"]["materialization_state"], "materialized");
    assert!(commit["commit"]["paths"]
        .as_array()
        .expect("commit paths")
        .iter()
        .any(|path| path["path"] == "docs/note.txt" && path["op"] == "create"));
    assert_eq!(
        fs::read_to_string(remote_root.join("docs/note.txt")).expect("read remote writer file"),
        "hello from writer"
    );

    let marker =
        fs::read(writer_root.join(".section/root.json")).expect("read writer marker after commit");
    let marker: Value = serde_json::from_slice(&marker).expect("marker after commit json");
    assert_eq!(marker["base_commit_id"], commit_id);

    let head_json =
        fs::read(remote_root.join(".section/agentfs/heads/current.json")).expect("read head");
    let head_json: Value = serde_json::from_slice(&head_json).expect("head json");
    assert_eq!(head_json["commit_id"], commit_id);

    let commit_json = fs::read(
        remote_root
            .join(".section/agentfs/commits")
            .join(format!("{commit_id}.json")),
    )
    .expect("read commit metadata");
    let commit_json: Value = serde_json::from_slice(&commit_json).expect("commit metadata json");
    assert_eq!(commit_json["materialization_state"], "materialized");

    let events = fs::read_dir(remote_root.join(".section/agentfs/events"))
        .expect("events dir")
        .map(|entry| {
            let entry = entry.expect("event entry");
            let event: Value = serde_json::from_slice(&fs::read(entry.path()).expect("read event"))
                .expect("event json");
            event["kind"].as_str().expect("kind").to_string()
        })
        .collect::<Vec<_>>();
    assert!(events.contains(&"commit.accepted".to_string()));
    assert!(events.contains(&"commit.materialized".to_string()));

    let owner_attach = run_section(
        &owner_config,
        &[
            "--json",
            "fs",
            "attach",
            "project",
            owner_root.to_str().expect("utf8 owner root"),
        ],
    );
    assert_success(&owner_attach, "owner attach after writer commit");
    assert_eq!(
        fs::read_to_string(owner_root.join("docs/note.txt"))
            .expect("owner reads materialized file"),
        "hello from writer"
    );

    let writer_reattach = run_section(
        &writer_config,
        &[
            "--json",
            "fs",
            "attach",
            "project",
            writer_second_root
                .to_str()
                .expect("utf8 second writer root"),
        ],
    );
    assert_success(&writer_reattach, "writer reattach to second empty root");
    assert_eq!(
        fs::read_to_string(writer_second_root.join("docs/note.txt"))
            .expect("writer second root reads materialized file"),
        "hello from writer"
    );
    assert_eq!(
        fs::read_to_string(remote_root.join("docs/note.txt")).expect("remote file after reattach"),
        "hello from writer"
    );
    assert!(
        !writer_root.join(".section/root.json").exists(),
        "reattach should remove previous root marker"
    );

    let downgrade = run_section(
        &owner_config,
        &[
            "--json", "fs", "grant", "project", &writer_id, "--role", "reader",
        ],
    );
    assert_success(&downgrade, "downgrade writer to reader");
    fs::write(
        writer_second_root.join("docs/after-downgrade.txt"),
        "should stay local",
    )
    .expect("write after downgrade");
    let denied_commit = run_section(
        &writer_config,
        &[
            "--json",
            "commit",
            "apply",
            writer_second_root
                .to_str()
                .expect("utf8 second writer root"),
            "--message",
            "should be denied",
        ],
    );
    assert!(
        !denied_commit.status.success(),
        "downgraded writer commit unexpectedly succeeded\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&denied_commit.stdout),
        String::from_utf8_lossy(&denied_commit.stderr)
    );
    let denied_commit: Value =
        serde_json::from_slice(&denied_commit.stdout).expect("denied commit json");
    assert_eq!(denied_commit["error"]["code"], "grant_denied");
    assert!(
        !remote_root.join("docs/after-downgrade.txt").exists(),
        "downgraded writer local draft must not materialize"
    );
}

#[test]
fn reader_can_attach_but_commit_apply_is_denied() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let remote_root = temp_dir.path().join("remote");
    let owner_data = temp_dir.path().join("owner-data");
    let reader_data = temp_dir.path().join("reader-data");
    let owner_config = temp_dir.path().join("owner.toml");
    let reader_config = temp_dir.path().join("reader.toml");
    let reader_root = temp_dir.path().join("reader-root");

    fs::create_dir_all(&remote_root).expect("create remote root");
    write_config(&owner_config, &owner_data);
    write_config(&reader_config, &reader_data);

    let owner_register = run_section(&owner_config, &["--json", "agent", "register", "owner"]);
    assert_success(&owner_register, "owner register");

    let create = run_section(
        &owner_config,
        &[
            "--json",
            "fs",
            "create",
            "project",
            "--provider",
            "fs",
            "--opt",
            &format!("root={}", remote_root.display()),
        ],
    );
    assert_success(&create, "owner fs create");

    let reader_register = run_section(&reader_config, &["--json", "agent", "register", "reader"]);
    assert_success(&reader_register, "reader register");
    let reader_register: Value =
        serde_json::from_slice(&reader_register.stdout).expect("reader register json");
    let reader_id = reader_register["agent"]["agent_id"]
        .as_str()
        .expect("reader id")
        .to_string();

    let reader_source = run_section(
        &reader_config,
        &[
            "source",
            "add",
            "project",
            "--provider",
            "fs",
            "--opt",
            &format!("root={}", remote_root.display()),
        ],
    );
    assert_success(&reader_source, "reader backing source add");

    let grant = run_section(
        &owner_config,
        &[
            "--json", "fs", "grant", "project", &reader_id, "--role", "reader",
        ],
    );
    assert_success(&grant, "grant reader");

    let attach = run_section(
        &reader_config,
        &[
            "--json",
            "fs",
            "attach",
            "project",
            reader_root.to_str().expect("utf8 reader root"),
        ],
    );
    assert_success(&attach, "reader attach");

    let reader_file = reader_root.join("draft.txt");
    fs::write(&reader_file, "reader local draft").expect("write reader draft");

    let commit = run_section(
        &reader_config,
        &[
            "--json",
            "commit",
            "apply",
            reader_root.to_str().expect("utf8 reader root"),
            "--message",
            "reader draft",
        ],
    );
    assert!(
        !commit.status.success(),
        "reader commit unexpectedly succeeded\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&commit.stdout),
        String::from_utf8_lossy(&commit.stderr)
    );
    let error: Value = serde_json::from_slice(&commit.stdout).expect("error json");
    assert_eq!(error["error"]["code"], "grant_denied");
    assert!(
        !remote_root.join("draft.txt").exists(),
        "reader draft must not materialize to shared truth"
    );
}

#[test]
fn fs_attach_rejects_non_empty_local_root_without_publishing_drafts() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let remote_root = temp_dir.path().join("remote");
    let owner_data = temp_dir.path().join("owner-data");
    let reader_data = temp_dir.path().join("reader-data");
    let owner_config = temp_dir.path().join("owner.toml");
    let reader_config = temp_dir.path().join("reader.toml");
    let reader_root = temp_dir.path().join("reader-root");

    fs::create_dir_all(&remote_root).expect("create remote root");
    fs::create_dir_all(&reader_root).expect("create reader root");
    fs::write(reader_root.join("draft.txt"), "must stay local").expect("write local draft");
    write_config(&owner_config, &owner_data);
    write_config(&reader_config, &reader_data);

    let owner_register = run_section(&owner_config, &["--json", "agent", "register", "owner"]);
    assert_success(&owner_register, "owner register");
    let create = run_section(
        &owner_config,
        &[
            "--json",
            "fs",
            "create",
            "project",
            "--provider",
            "fs",
            "--opt",
            &format!("root={}", remote_root.display()),
        ],
    );
    assert_success(&create, "owner fs create");

    let reader_register = run_section(&reader_config, &["--json", "agent", "register", "reader"]);
    assert_success(&reader_register, "reader register");
    let reader_register: Value =
        serde_json::from_slice(&reader_register.stdout).expect("reader register json");
    let reader_id = reader_register["agent"]["agent_id"]
        .as_str()
        .expect("reader id")
        .to_string();

    let reader_source = run_section(
        &reader_config,
        &[
            "source",
            "add",
            "project",
            "--provider",
            "fs",
            "--opt",
            &format!("root={}", remote_root.display()),
        ],
    );
    assert_success(&reader_source, "reader backing source add");

    let grant = run_section(
        &owner_config,
        &[
            "--json", "fs", "grant", "project", &reader_id, "--role", "reader",
        ],
    );
    assert_success(&grant, "grant reader");

    let attach = run_section(
        &reader_config,
        &[
            "--json",
            "fs",
            "attach",
            "project",
            reader_root.to_str().expect("utf8 reader root"),
        ],
    );
    assert!(
        !attach.status.success(),
        "attach unexpectedly succeeded\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&attach.stdout),
        String::from_utf8_lossy(&attach.stderr)
    );
    assert!(
        !remote_root.join("draft.txt").exists(),
        "attach must not publish pre-existing local drafts"
    );
    assert!(
        !reader_root.join(".section/root.json").exists(),
        "failed attach must not leave an AgentFS marker"
    );
}
