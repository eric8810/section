mod support;

use crate::support::{assert_success, run_section, write_agentfs_config};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FS_NAME: &str = "project";
const SOURCE_PROFILE: &str = "test-profile";

fn test_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|shell| shell.starts_with('/') && Path::new(shell).exists())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

struct AgentFsFixture {
    _temp_dir: tempfile::TempDir,
    root: PathBuf,
    remote_root: PathBuf,
    control_service_path: PathBuf,
}

impl AgentFsFixture {
    fn new() -> Self {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let root = temp_dir.path().to_path_buf();
        let remote_root = root.join("remote");
        let control_service_path = root.join("control-service.sqlite");
        fs::create_dir_all(&remote_root).expect("create remote root");
        Self {
            _temp_dir: temp_dir,
            root,
            remote_root,
            control_service_path,
        }
    }

    fn agent(&self, name: &str) -> AgentFsActor {
        let data_dir = self.root.join(format!("{name}-data"));
        let config_path = self.root.join(format!("{name}.toml"));
        write_agentfs_config(
            &config_path,
            &data_dir,
            &self.control_service_path,
            SOURCE_PROFILE,
            &self.remote_root,
        );
        AgentFsActor {
            config_path,
            data_dir,
            local_root: self.root.join(format!("{name}-root")),
        }
    }

    fn path(&self, relative_path: &str) -> PathBuf {
        self.remote_root.join(relative_path)
    }

    fn set_fs_source_name(&self, fs_id: &str, source_name: &str) {
        let conn = rusqlite::Connection::open(&self.control_service_path).expect("control db");
        conn.execute(
            "UPDATE filesystems SET source_name = ?1 WHERE fs_id = ?2",
            rusqlite::params![source_name, fs_id],
        )
        .expect("update source_name");
    }

    fn insert_duplicate_source_name(&self, fs_id: &str, name: &str, source_name: &str) {
        let conn = rusqlite::Connection::open(&self.control_service_path).expect("control db");
        let (owner_agent_id, source_profile_id): (String, String) = conn
            .query_row(
                "SELECT owner_agent_id, source_profile_id FROM filesystems LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read existing fs fields");
        conn.execute(
            "INSERT INTO filesystems (
                fs_id, name, owner_agent_id, source_profile_id, source_name, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, 1)",
            rusqlite::params![fs_id, name, owner_agent_id, source_profile_id, source_name],
        )
        .expect("insert duplicate source_name fs");
    }

    fn control_row_count(&self, table: &str) -> i64 {
        let table = match table {
            "filesystems" | "grants" | "events" | "shares" | "credential_bindings" => table,
            other => panic!("unsupported control table {other}"),
        };
        let conn = rusqlite::Connection::open(&self.control_service_path).expect("control db");
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("count control rows")
    }
}

struct AgentFsActor {
    config_path: PathBuf,
    data_dir: PathBuf,
    local_root: PathBuf,
}

impl AgentFsActor {
    fn login(&self, name: &str) -> String {
        let output = self.json_owned(vec![
            "--json".to_string(),
            "agent".to_string(),
            "login".to_string(),
            name.to_string(),
        ]);
        output["agent"]["agent_id"]
            .as_str()
            .expect("agent id")
            .to_string()
    }

    fn create_fs(&self) -> Value {
        let output = self.create_fs_output();
        assert_success(&output, "fs create");
        serde_json::from_slice(&output.stdout).expect("fs create json")
    }

    fn create_fs_output(&self) -> Output {
        self.run_owned(vec![
            "--json".to_string(),
            "fs".to_string(),
            "create".to_string(),
            FS_NAME.to_string(),
            "--source-profile".to_string(),
            SOURCE_PROFILE.to_string(),
        ])
    }

    fn list_sources(&self) -> Value {
        self.json(&["--json", "source", "list"], "source list")
    }

    fn grant(&self, agent_id: &str, role: &str) -> Value {
        self.grant_with_scopes(agent_id, role, &[])
    }

    fn grant_with_scopes(&self, agent_id: &str, role: &str, scopes: &[&str]) -> Value {
        let mut args = vec![
            "--json".to_string(),
            "fs".to_string(),
            "grant".to_string(),
            FS_NAME.to_string(),
            agent_id.to_string(),
            "--role".to_string(),
            role.to_string(),
        ];
        for scope in scopes {
            args.push("--scope".to_string());
            args.push((*scope).to_string());
        }
        self.json_owned(args)
    }

    fn revoke(&self, agent_id: &str) -> Value {
        self.json_owned(vec![
            "--json".to_string(),
            "fs".to_string(),
            "revoke".to_string(),
            FS_NAME.to_string(),
            agent_id.to_string(),
        ])
    }

    fn revoke_output(&self, agent_id: &str) -> Output {
        self.run_owned(vec![
            "--json".to_string(),
            "fs".to_string(),
            "revoke".to_string(),
            FS_NAME.to_string(),
            agent_id.to_string(),
        ])
    }

    fn share(&self, agent_id: &str) -> Value {
        self.json_owned(vec![
            "--json".to_string(),
            "fs".to_string(),
            "share".to_string(),
            FS_NAME.to_string(),
            agent_id.to_string(),
        ])
    }

    fn available(&self) -> Value {
        self.json(&["--json", "fs", "available"], "fs available")
    }

    fn accept(&self, share_id: &str) -> Value {
        self.json_owned(vec![
            "--json".to_string(),
            "fs".to_string(),
            "accept".to_string(),
            share_id.to_string(),
        ])
    }

    fn attach(&self) -> Value {
        self.attach_to(&self.local_root)
    }

    fn attach_to(&self, local_root: &Path) -> Value {
        let output = self.attach_output(local_root);
        assert_success(&output, "fs attach");
        serde_json::from_slice(&output.stdout).expect("attach json")
    }

    fn attach_output(&self, local_root: &Path) -> Output {
        self.run_owned(vec![
            "--json".to_string(),
            "fs".to_string(),
            "attach".to_string(),
            FS_NAME.to_string(),
            local_root.to_str().expect("utf8 local root").to_string(),
        ])
    }

    fn commit_status(&self) -> Value {
        self.json_owned(vec![
            "--json".to_string(),
            "commit".to_string(),
            "status".to_string(),
            self.local_root
                .to_str()
                .expect("utf8 local root")
                .to_string(),
        ])
    }

    fn fs_status(&self, fs_ref: &str) -> Value {
        let output = self.fs_status_output(fs_ref);
        assert_success(&output, "fs status");
        serde_json::from_slice(&output.stdout).expect("status json")
    }

    fn fs_status_output(&self, fs_ref: &str) -> Output {
        self.run_owned(vec![
            "--json".to_string(),
            "fs".to_string(),
            "status".to_string(),
            fs_ref.to_string(),
        ])
    }

    fn commit_apply(&self, message: &str) -> Value {
        let output = self.commit_apply_output(message);
        assert_success(&output, "commit apply");
        serde_json::from_slice(&output.stdout).expect("commit json")
    }

    fn commit_apply_with_env(&self, message: &str, envs: &[(&str, &str)]) -> Value {
        let output = self.run_owned_with_env(
            vec![
                "--json".to_string(),
                "commit".to_string(),
                "apply".to_string(),
                self.local_root
                    .to_str()
                    .expect("utf8 local root")
                    .to_string(),
                "--message".to_string(),
                message.to_string(),
            ],
            envs,
        );
        assert_success(&output, "commit apply");
        serde_json::from_slice(&output.stdout).expect("commit json")
    }

    fn commit_apply_output(&self, message: &str) -> Output {
        self.run_owned(vec![
            "--json".to_string(),
            "commit".to_string(),
            "apply".to_string(),
            self.local_root
                .to_str()
                .expect("utf8 local root")
                .to_string(),
            "--message".to_string(),
            message.to_string(),
        ])
    }

    fn commit_repair(&self, fs_ref: &str, commit_id: &str) -> Value {
        self.json_owned(vec![
            "--json".to_string(),
            "commit".to_string(),
            "repair".to_string(),
            fs_ref.to_string(),
            "--commit".to_string(),
            commit_id.to_string(),
        ])
    }

    fn commit_propose(&self, message: &str) -> Value {
        self.json_owned(vec![
            "--json".to_string(),
            "commit".to_string(),
            "propose".to_string(),
            self.local_root
                .to_str()
                .expect("utf8 local root")
                .to_string(),
            "--message".to_string(),
            message.to_string(),
        ])
    }

    fn commit_accept(&self, fs_ref: &str, proposal_id: &str) -> Value {
        self.json_owned(vec![
            "--json".to_string(),
            "commit".to_string(),
            "accept".to_string(),
            fs_ref.to_string(),
            proposal_id.to_string(),
        ])
    }

    fn commit_accept_output(&self, fs_ref: &str, proposal_id: &str) -> Output {
        self.run_owned(vec![
            "--json".to_string(),
            "commit".to_string(),
            "accept".to_string(),
            fs_ref.to_string(),
            proposal_id.to_string(),
        ])
    }

    fn commit_reject(&self, fs_ref: &str, proposal_id: &str) -> Value {
        self.json_owned(vec![
            "--json".to_string(),
            "commit".to_string(),
            "reject".to_string(),
            fs_ref.to_string(),
            proposal_id.to_string(),
        ])
    }

    fn fs_events(&self, fs_ref: &str, after: Option<&str>) -> Value {
        let output = self.fs_events_output(fs_ref, after);
        assert_success(&output, "fs events");
        serde_json::from_slice(&output.stdout).expect("events json")
    }

    fn fs_events_output(&self, fs_ref: &str, after: Option<&str>) -> Output {
        let mut args = vec![
            "--json".to_string(),
            "fs".to_string(),
            "events".to_string(),
            fs_ref.to_string(),
        ];
        if let Some(after) = after {
            args.push("--after".to_string());
            args.push(after.to_string());
        }
        self.run_owned(args)
    }

    fn hooks_add(&self, fs_ref: &str, name: &str, command: &[&str]) -> Value {
        let output = self.hooks_add_output(fs_ref, name, command);
        assert_success(&output, "hooks add");
        serde_json::from_slice(&output.stdout).expect("hooks add json")
    }

    fn hooks_add_output(&self, fs_ref: &str, name: &str, command: &[&str]) -> Output {
        let mut args = vec![
            "--json".to_string(),
            "hooks".to_string(),
            "add".to_string(),
            fs_ref.to_string(),
            "--name".to_string(),
            name.to_string(),
            "--".to_string(),
        ];
        args.extend(command.iter().map(|value| value.to_string()));
        self.run_owned(args)
    }

    fn hooks_list(&self, fs_ref: &str) -> Value {
        self.json_owned(vec![
            "--json".to_string(),
            "hooks".to_string(),
            "list".to_string(),
            fs_ref.to_string(),
        ])
    }

    fn hooks_remove(&self, fs_ref: &str, hook_id: &str) -> Value {
        self.json_owned(vec![
            "--json".to_string(),
            "hooks".to_string(),
            "remove".to_string(),
            fs_ref.to_string(),
            hook_id.to_string(),
        ])
    }

    fn hook_runs(&self, fs_id: &str) -> Vec<Value> {
        let conn = rusqlite::Connection::open(self.data_dir.join("section.db"))
            .expect("open local section db");
        let mut stmt = conn
            .prepare(
                "SELECT run_id, hook_id, fs_id, event_id, status, exit_code,
                        stdout_tail, stderr_tail
                 FROM agentfs_hook_runs
                 WHERE fs_id = ?1
                 ORDER BY started_at_ms, run_id",
            )
            .expect("prepare hook runs query");
        stmt.query_map([fs_id], |row| {
            Ok(json!({
                "run_id": row.get::<_, String>(0)?,
                "hook_id": row.get::<_, String>(1)?,
                "fs_id": row.get::<_, String>(2)?,
                "event_id": row.get::<_, String>(3)?,
                "status": row.get::<_, String>(4)?,
                "exit_code": row.get::<_, Option<i32>>(5)?,
                "stdout_tail": row.get::<_, String>(6)?,
                "stderr_tail": row.get::<_, String>(7)?,
            }))
        })
        .expect("query hook runs")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect hook runs")
    }

    fn watch_agentfs_once(&self, fs_ref: &Path) -> Vec<Value> {
        let output = self.run_owned(vec![
            "--json".to_string(),
            "watch".to_string(),
            fs_ref.to_str().expect("utf8 fs ref").to_string(),
            "--agentfs".to_string(),
            "--once".to_string(),
        ]);
        assert_success(&output, "watch --agentfs --once");
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| serde_json::from_str(line).expect("watch event json"))
            .collect()
    }

    fn write_local(&self, relative_path: &str, content: &str) {
        let path = self.local_root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create local parent");
        }
        fs::write(path, content).expect("write local file");
    }

    fn run_owned(&self, args: Vec<String>) -> Output {
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();
        run_section(&self.config_path, &args)
    }

    fn run_owned_with_env(&self, args: Vec<String>, envs: &[(&str, &str)]) -> Output {
        let bin = env!("CARGO_BIN_EXE_section");
        let mut command = Command::new(bin);
        command.arg("--config").arg(&self.config_path).args(args);
        for (key, value) in envs {
            command.env(key, value);
        }
        command.output().expect("run section")
    }

    fn json(&self, args: &[&str], context: &str) -> Value {
        let output = run_section(&self.config_path, args);
        assert_success(&output, context);
        serde_json::from_slice(&output.stdout).expect("json output")
    }

    fn json_owned(&self, args: Vec<String>) -> Value {
        let output = self.run_owned(args);
        assert_success(&output, "json command");
        serde_json::from_slice(&output.stdout).expect("json output")
    }
}

fn assert_json_error(output: &Output, code: &str) -> Value {
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let error: Value = serde_json::from_slice(&output.stdout).expect("error json");
    assert_eq!(error["error"]["code"], code);
    assert!(
        error["error"]["retryable"].is_boolean(),
        "error retryable must be a boolean: {error:?}"
    );
    assert!(
        error["error"]["details"].is_object(),
        "error details must be an object: {error:?}"
    );
    error
}

fn read_json(path: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&fs::read(path).expect("read json")).expect("parse json")
}

fn write_json(path: impl AsRef<Path>, value: &Value) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize json"),
    )
    .expect("write json");
}

fn agentfs_event_kinds(remote_root: &Path) -> Vec<String> {
    let mut kinds = fs::read_dir(remote_root.join(".section/agentfs/events"))
        .expect("events dir")
        .map(|entry| {
            let entry = entry.expect("event entry");
            let event = read_json(entry.path());
            event["kind"].as_str().expect("event kind").to_string()
        })
        .collect::<Vec<_>>();
    kinds.sort();
    kinds
}

#[test]
fn e2e_writer_commit_becomes_shared_truth_for_owner() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    let writer = fixture.agent("writer");

    let owner_id = owner.login("owner");
    let missing_source_profile = owner.run_owned(vec![
        "--json".to_string(),
        "fs".to_string(),
        "create".to_string(),
        "missing-source-profile".to_string(),
    ]);
    let missing_source_profile_error =
        assert_json_error(&missing_source_profile, "operation_failed");
    assert_eq!(missing_source_profile_error["error"]["retryable"], false);
    assert_eq!(
        missing_source_profile_error["error"]["details"]["command"],
        "fs"
    );
    let invalid_args = owner.run_owned(vec![
        "--json".to_string(),
        "fs".to_string(),
        "status".to_string(),
    ]);
    let invalid_args_error = assert_json_error(&invalid_args, "invalid_arguments");
    assert!(invalid_args_error["error"]["details"]["kind"].is_string());
    let missing_watch = owner.run_owned(vec![
        "--json".to_string(),
        "watch".to_string(),
        "missing-agentfs".to_string(),
        "--agentfs".to_string(),
        "--once".to_string(),
    ]);
    let missing_watch_error = assert_json_error(&missing_watch, "unknown_fs");
    assert_eq!(
        missing_watch_error["error"]["details"]["reference"],
        "missing-agentfs"
    );
    let create = owner.create_fs();
    let fs_id = create["fs"]["fs_id"].as_str().expect("fs id").to_string();
    assert_eq!(create["fs"]["owner_agent_id"], owner_id);
    owner.attach();

    let writer_id = writer.login("writer");
    let grant = owner.grant(&writer_id, "writer");
    assert_eq!(grant["grant"]["agent_id"], writer_id);
    assert_eq!(grant["grant"]["role"], "writer");
    let share = owner.share(&writer_id);
    let share_id = share["share"]["share_id"]
        .as_str()
        .expect("share id")
        .to_string();
    let available = writer.available();
    assert!(available["available"]
        .as_array()
        .expect("available shares")
        .iter()
        .any(|share| share["share"]["share_id"] == share_id));
    let accept = writer.accept(&share_id);
    assert_eq!(accept["fs"]["fs_id"], fs_id);
    let credential = &accept["credential_binding"];
    assert!(credential["credential_binding_id"]
        .as_str()
        .expect("credential binding id")
        .starts_with("cred_"));
    assert_eq!(credential["fs_id"], fs_id);
    assert_eq!(credential["agent_id"], writer_id);
    assert!(credential["installation_id"]
        .as_str()
        .expect("installation id")
        .starts_with("ins_"));
    assert_eq!(
        credential["source_profile_id"],
        accept["fs"]["source_profile_id"]
    );
    assert!(
        credential["expires_at_ms"].as_i64().expect("expires")
            > credential["issued_at_ms"].as_i64().expect("issued"),
        "credential binding must be short-lived with an expiry"
    );
    assert!(
        !serde_json::to_string(&accept)
            .expect("accept json string")
            .contains(fixture.remote_root.to_str().expect("utf8 remote root")),
        "accept JSON must not expose backing source paths"
    );
    let credential_count_after_accept = fixture.control_row_count("credential_bindings");

    let attach = writer.attach();
    let credential_count_after_attach = fixture.control_row_count("credential_bindings");
    assert!(
        credential_count_after_attach > credential_count_after_accept,
        "attach must refresh a service-issued credential before touching backing source"
    );
    assert_eq!(attach["attach"]["fs"]["fs_id"], fs_id);
    assert!(
        attach["attach"]["source"]["options"].is_null(),
        "attach JSON must not expose source options"
    );
    assert!(
        !serde_json::to_string(&attach)
            .expect("attach json string")
            .contains(fixture.remote_root.to_str().expect("utf8 remote root")),
        "attach JSON must not expose backing source paths"
    );

    writer.write_local("docs/note.txt", "hello from writer");
    assert!(
        !fixture.path("docs/note.txt").exists(),
        "local edit must not be shared truth before commit"
    );

    let status = writer.commit_status();
    let credential_count_after_status = fixture.control_row_count("credential_bindings");
    assert!(
        credential_count_after_status > credential_count_after_attach,
        "commit status must refresh a service-issued credential before reading backing source"
    );
    let dirty_paths = status["status"]["dirty_paths"]
        .as_array()
        .expect("dirty paths");
    assert!(
        dirty_paths
            .iter()
            .any(|path| path["path"] == "docs/note.txt" && path["op"] == "create"),
        "dirty paths should include writer note: {dirty_paths:?}"
    );

    let commit = writer.commit_apply("add writer note");
    assert!(
        fixture.control_row_count("credential_bindings") > credential_count_after_status,
        "commit apply must refresh a service-issued credential before accepting shared truth"
    );
    let commit_id = commit["commit"]["commit_id"]
        .as_str()
        .expect("commit id")
        .to_string();
    assert_eq!(commit["commit"]["agent_id"], writer_id);
    assert_eq!(commit["commit"]["materialization_state"], "materialized");
    assert_eq!(
        fs::read_to_string(fixture.path("docs/note.txt")).expect("read remote note"),
        "hello from writer"
    );

    let head = read_json(
        fixture
            .remote_root
            .join(".section/agentfs/heads/current.json"),
    );
    assert_eq!(head["commit_id"], commit_id);
    let commit_record = read_json(
        fixture
            .remote_root
            .join(".section/agentfs/commits")
            .join(format!("{commit_id}.json")),
    );
    assert_eq!(commit_record["agent_id"], writer_id);
    assert_eq!(commit_record["base_commit_id"], Value::Null);
    let staging_manifest_path = commit_record["staging_snapshot"]["manifest_path"]
        .as_str()
        .expect("staging manifest path");
    assert!(
        Path::new(staging_manifest_path).exists(),
        "staging manifest must exist at {staging_manifest_path}"
    );
    assert!(commit_record["paths"]
        .as_array()
        .expect("commit paths")
        .iter()
        .any(|path| path["path"] == "docs/note.txt" && path["op"] == "create"));

    let events = agentfs_event_kinds(&fixture.remote_root);
    assert!(events.contains(&"fs.created".to_string()));
    assert!(events.contains(&"grant.created".to_string()));
    assert!(events.contains(&"commit.accepted".to_string()));
    assert!(events.contains(&"commit.materialized".to_string()));

    let replay = writer.fs_events(FS_NAME, None);
    let replay_events = replay["events"].as_array().expect("replay events");
    assert!(
        replay_events.len() >= 4,
        "expected core AgentFS events, got {replay_events:?}"
    );
    let mut last_seq = 0;
    for event in replay_events {
        let seq = event["seq"].as_i64().expect("event seq");
        assert!(seq > last_seq, "event seq must increase: {replay_events:?}");
        last_seq = seq;
    }
    let accepted_event = replay_events
        .iter()
        .find(|event| event["kind"] == "commit.accepted")
        .expect("commit.accepted event");
    assert_eq!(
        accepted_event["data"]["authorized_by"]["type"], "grant",
        "writer commit should explain grant authority"
    );
    assert_eq!(
        accepted_event["data"]["authorized_by"]["grant_id"],
        grant["grant"]["grant_id"]
    );
    let materialized_event = replay_events
        .iter()
        .find(|event| event["kind"] == "commit.materialized")
        .expect("commit.materialized event");
    assert_eq!(materialized_event["subject_id"], commit_id);
    assert_eq!(
        materialized_event["data"]["materialization_state"],
        "materialized"
    );
    assert!(materialized_event["data"]["paths"]
        .as_array()
        .expect("materialized paths")
        .iter()
        .any(|path| path["path"] == "docs/note.txt"
            && path["op"] == "create"
            && path["kind"] == "file"));

    let first_seq = replay_events[0]["seq"].as_i64().expect("first seq");
    let after = writer.fs_events(FS_NAME, Some(&first_seq.to_string()));
    assert!(after["events"]
        .as_array()
        .expect("after events")
        .iter()
        .all(|event| event["seq"].as_i64().expect("after seq") > first_seq));

    let watched = writer.watch_agentfs_once(&writer.local_root);
    assert!(
        watched.iter().any(|line| {
            line["stream"] == "agentfs" && line["event"]["kind"] == "commit.accepted"
        }),
        "watch --agentfs should expose commit.accepted, got {watched:?}"
    );

    writer.write_local("docs/note.txt", "writer next draft");
    let local_after_commit_status = writer.commit_status();
    assert!(
        local_after_commit_status["status"]["dirty_paths"]
            .as_array()
            .expect("dirty paths after local draft")
            .iter()
            .any(|path| path["path"] == "docs/note.txt"),
        "live local edits after commit should remain dirty work"
    );

    fs::write(fixture.path("docs/note.txt"), "broken remote").expect("corrupt remote");
    let commit_path = fixture
        .remote_root
        .join(".section/agentfs/commits")
        .join(format!("{commit_id}.json"));
    let mut failed_commit = read_json(&commit_path);
    failed_commit["materialization_state"] = Value::String("failed_to_materialize".to_string());
    failed_commit["error"] = Value::String("forced failure for repair test".to_string());
    write_json(&commit_path, &failed_commit);

    writer.write_local("docs/blocked.txt", "blocked while head failed");
    let blocked = writer.commit_apply_output("blocked by failed head");
    assert_json_error(&blocked, "materialization_failed");

    let repaired = writer.commit_repair(&writer.local_root.to_string_lossy(), &commit_id);
    assert_eq!(repaired["commit"]["commit_id"], commit_id);
    assert_eq!(repaired["commit"]["materialization_state"], "materialized");
    assert_eq!(
        fs::read_to_string(fixture.path("docs/note.txt")).expect("read repaired remote note"),
        "hello from writer",
        "repair must materialize the original staging snapshot"
    );

    let after_repair_commit = writer.commit_apply("commit after repair");
    assert_eq!(
        after_repair_commit["commit"]["parent_commit_id"], commit_id,
        "new commit after repair must build on the repaired head"
    );
    assert_eq!(
        fs::read_to_string(fixture.path("docs/blocked.txt"))
            .expect("read post-repair committed file"),
        "blocked while head failed"
    );

    let owner_fresh_root = fixture.root.join("owner-fresh-root");
    owner.attach_to(&owner_fresh_root);
    assert_eq!(
        fs::read_to_string(owner_fresh_root.join("docs/note.txt"))
            .expect("owner reads accepted writer content"),
        "writer next draft"
    );
}

#[test]
fn e2e_fs_ref_resolves_source_name_and_rejects_ambiguity() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    let create = owner.create_fs();
    let fs_id = create["fs"]["fs_id"].as_str().expect("fs id").to_string();

    fixture.set_fs_source_name(&fs_id, "legacy-source");
    let status_by_source_name = owner.fs_status("legacy-source");
    assert_eq!(status_by_source_name["status"]["fs"]["fs_id"], fs_id);

    fixture.insert_duplicate_source_name(
        "fs_deadbeefdeadbeefdeadbeefdeadbeef",
        "other",
        "legacy-source",
    );
    let ambiguous = owner.fs_status_output("legacy-source");
    let error = assert_json_error(&ambiguous, "ambiguous_fs_ref");
    assert_eq!(error["error"]["details"]["reference"], "legacy-source");
    assert_eq!(error["error"]["details"]["matched_field"], "source_name");
    assert_eq!(
        error["error"]["details"]["matches"]
            .as_array()
            .expect("matches")
            .len(),
        2
    );

    let status_by_fs_id = owner.fs_status(&fs_id);
    assert_eq!(status_by_fs_id["status"]["fs"]["fs_id"], fs_id);
}

#[test]
fn e2e_bad_metadata_in_unrelated_source_does_not_block_fs_lookup() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    let create = owner.create_fs();
    let fs_id = create["fs"]["fs_id"].as_str().expect("fs id").to_string();

    let bad_source_root = fixture.root.join("bad-source");
    fs::create_dir_all(bad_source_root.join(".section/agentfs"))
        .expect("create bad source metadata dir");
    fs::write(
        bad_source_root.join(".section/agentfs/fs.json"),
        "{not valid json",
    )
    .expect("write malformed unrelated fs metadata");
    let add_bad_source = owner.run_owned(vec![
        "--json".to_string(),
        "source".to_string(),
        "add".to_string(),
        "bad-source".to_string(),
        "--provider".to_string(),
        "fs".to_string(),
        "--opt".to_string(),
        format!("root={}", bad_source_root.display()),
    ]);
    assert_success(&add_bad_source, "source add bad metadata source");

    let status = owner.fs_status(FS_NAME);
    assert_eq!(status["status"]["fs"]["fs_id"], fs_id);
    let events = owner.fs_events(FS_NAME, None);
    assert!(events["events"]
        .as_array()
        .expect("events")
        .iter()
        .any(|event| event["kind"] == "fs.created"));
}

#[test]
fn e2e_rejects_invalid_shared_metadata_schema_and_links() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    let create = owner.create_fs();
    let fs_id = create["fs"]["fs_id"].as_str().expect("fs id").to_string();
    owner.attach();
    owner.write_local("docs/schema.txt", "schema");
    let commit = owner.commit_apply("create schema file");
    let commit_id = commit["commit"]["commit_id"]
        .as_str()
        .expect("commit id")
        .to_string();

    let commit_path = fixture
        .remote_root
        .join(".section/agentfs/commits")
        .join(format!("{commit_id}.json"));
    let original_commit = read_json(&commit_path);
    let mut bad_commit = original_commit.clone();
    bad_commit["schema_version"] = Value::from(999);
    write_json(&commit_path, &bad_commit);
    let bad_commit_status = owner.fs_status_output(&owner.local_root.to_string_lossy());
    assert_json_error(&bad_commit_status, "malformed_shared_metadata");
    write_json(&commit_path, &original_commit);

    let head_path = fixture
        .remote_root
        .join(".section/agentfs/heads/current.json");
    let original_head = read_json(&head_path);
    let mut wrong_head = original_head.clone();
    wrong_head["fs_id"] = Value::String("fs_11111111111111111111111111111111".to_string());
    write_json(&head_path, &wrong_head);
    let wrong_head_status = owner.fs_status_output(FS_NAME);
    assert_json_error(&wrong_head_status, "malformed_shared_metadata");
    write_json(&head_path, &original_head);

    let event_path = fs::read_dir(fixture.remote_root.join(".section/agentfs/events"))
        .expect("events dir")
        .map(|entry| entry.expect("event entry").path())
        .find(|path| read_json(path)["fs_id"].as_str().expect("event fs_id") == fs_id)
        .expect("event path");
    let original_event = read_json(&event_path);
    let mut wrong_event = original_event.clone();
    wrong_event["fs_id"] = Value::String("fs_22222222222222222222222222222222".to_string());
    write_json(&event_path, &wrong_event);
    let wrong_event_replay = owner.run_owned(vec![
        "--json".to_string(),
        "fs".to_string(),
        "events".to_string(),
        FS_NAME.to_string(),
    ]);
    assert_json_error(&wrong_event_replay, "malformed_shared_metadata");
}

#[test]
fn e2e_grants_control_attach_manage_and_commit_authority() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    let reader = fixture.agent("reader");
    let stranger = fixture.agent("stranger");
    let writer = fixture.agent("writer");
    let manager = fixture.agent("manager");

    owner.login("owner");
    owner.create_fs();

    let reader_id = reader.login("reader");
    owner.grant(&reader_id, "reader");
    let reader_share = owner.share(&reader_id);
    let reader_share_id = reader_share["share"]["share_id"]
        .as_str()
        .expect("share id");
    reader.accept(reader_share_id);
    reader.attach();
    reader.write_local("reader-draft.txt", "reader local draft");
    let reader_status = reader.fs_status(&reader.local_root.to_string_lossy());
    assert_eq!(reader_status["status"]["role"], "reader");
    assert!(reader.fs_events(FS_NAME, None)["events"]
        .as_array()
        .expect("reader events")
        .iter()
        .any(|event| event["kind"] == "fs.created"));
    assert_eq!(reader_status["status"]["dirty"], true);
    assert_eq!(reader_status["status"]["dirty_count"], 1);
    assert!(reader_status["status"]["next_actions"]
        .as_array()
        .expect("reader next actions")
        .iter()
        .any(|action| action == "request_grant"));
    assert!(!reader_status["status"]["next_actions"]
        .as_array()
        .expect("reader next actions")
        .iter()
        .any(|action| action == "commit"));
    assert_json_error(&reader.commit_apply_output("reader draft"), "grant_denied");
    assert!(
        !fixture.path("reader-draft.txt").exists(),
        "reader local draft must not become shared truth"
    );

    let stranger_id = stranger.login("stranger");
    assert_json_error(
        &stranger.attach_output(&stranger.local_root),
        "grant_denied",
    );
    assert!(
        !stranger.local_root.join(".section/root.json").exists(),
        "failed ungranted attach must not write root marker"
    );
    assert_json_error(&stranger.fs_status_output(FS_NAME), "grant_denied");
    assert_json_error(&stranger.fs_events_output(FS_NAME, None), "grant_denied");

    let writer_id = writer.login("writer");
    owner.grant(&writer_id, "writer");
    let writer_share = owner.share(&writer_id);
    let writer_share_id = writer_share["share"]["share_id"]
        .as_str()
        .expect("share id");
    writer.accept(writer_share_id);
    writer.attach();
    writer.write_local("writer-before-downgrade.txt", "writer committable draft");
    let writer_status = writer.fs_status(&writer.local_root.to_string_lossy());
    assert_eq!(writer_status["status"]["role"], "writer");
    assert_eq!(writer_status["status"]["dirty"], true);
    assert!(writer_status["status"]["capabilities"]
        .as_array()
        .expect("writer capabilities")
        .iter()
        .any(|capability| capability == "commit"));
    assert!(writer_status["status"]["next_actions"]
        .as_array()
        .expect("writer next actions")
        .iter()
        .any(|action| action == "commit"));
    owner.grant(&writer_id, "reader");
    assert_json_error(
        &writer.commit_apply_output("writer after downgrade"),
        "grant_denied",
    );
    assert!(
        !fixture.path("writer-before-downgrade.txt").exists(),
        "downgraded writer draft must not become shared truth"
    );

    let manager_id = manager.login("manager");
    owner.grant(&manager_id, "manager");
    let manager_share = owner.share(&manager_id);
    let manager_share_id = manager_share["share"]["share_id"]
        .as_str()
        .expect("share id");
    manager.accept(manager_share_id);
    let manager_grant = manager.grant(&stranger_id, "reader");
    assert_eq!(manager_grant["grant"]["agent_id"], stranger_id);
    assert_eq!(manager_grant["grant"]["role"], "reader");
    let managed_share = manager.share(&stranger_id);
    let managed_share_id = managed_share["share"]["share_id"]
        .as_str()
        .expect("manager-created share id");
    let stranger_available = stranger.available();
    assert!(stranger_available["available"]
        .as_array()
        .expect("stranger available shares")
        .iter()
        .any(|share| share["share"]["share_id"] == managed_share_id));
    stranger.accept(managed_share_id);
    stranger.attach();
    let stranger_status = stranger.fs_status(&stranger.local_root.to_string_lossy());
    assert_eq!(stranger_status["status"]["role"], "reader");
    let manager_revoke = manager.revoke(&stranger_id);
    assert_eq!(
        manager_revoke["revoked"][0]["agent_id"], stranger_id,
        "manager should be able to revoke grants"
    );
    manager.attach();
    manager.write_local("manager-draft.txt", "manager local draft");
    let manager_status = manager.fs_status(&manager.local_root.to_string_lossy());
    assert_eq!(manager_status["status"]["role"], "manager");
    assert!(manager_status["status"]["capabilities"]
        .as_array()
        .expect("manager capabilities")
        .iter()
        .any(|capability| capability == "manage"));
    assert!(!manager_status["status"]["capabilities"]
        .as_array()
        .expect("manager capabilities")
        .iter()
        .any(|capability| capability == "commit"));
    assert!(manager_status["status"]["next_actions"]
        .as_array()
        .expect("manager next actions")
        .iter()
        .any(|action| action == "request_grant"));
    assert_json_error(
        &manager.commit_apply_output("manager draft"),
        "grant_denied",
    );
    assert!(
        !fixture.path("manager-draft.txt").exists(),
        "manager draft must not become shared truth without commit capability"
    );
}

#[test]
fn e2e_revoke_removes_commit_access_and_blocks_pending_share_accept() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    let writer = fixture.agent("writer");
    let pending = fixture.agent("pending");

    let owner_id = owner.login("owner");
    owner.create_fs();

    let writer_id = writer.login("writer");
    owner.grant(&writer_id, "writer");
    let writer_share = owner.share(&writer_id);
    let writer_share_id = writer_share["share"]["share_id"]
        .as_str()
        .expect("writer share id");
    writer.accept(writer_share_id);
    writer.attach();

    let revoked = owner.revoke(&writer_id);
    assert_eq!(revoked["revoked"][0]["agent_id"], writer_id);
    let events = owner.fs_events(FS_NAME, None);
    assert!(events["events"]
        .as_array()
        .expect("events")
        .iter()
        .any(|event| event["kind"] == "grant.revoked" && event["data"]["agent_id"] == writer_id));

    writer.write_local("docs/revoked.txt", "must stay local");
    assert_json_error(
        &writer.commit_apply_output("writer after revoke"),
        "grant_denied",
    );
    assert!(
        !fixture.path("docs/revoked.txt").exists(),
        "revoked writer draft must not become shared truth"
    );

    assert_json_error(&owner.revoke_output(&owner_id), "grant_denied");

    let pending_id = pending.login("pending");
    owner.grant(&pending_id, "writer");
    let pending_share = owner.share(&pending_id);
    let pending_share_id = pending_share["share"]["share_id"]
        .as_str()
        .expect("pending share id")
        .to_string();
    owner.revoke(&pending_id);

    let available_after_revoke = pending.available();
    assert!(!available_after_revoke["available"]
        .as_array()
        .expect("available shares")
        .iter()
        .any(|share| share["share"]["share_id"] == pending_share_id));
    let accept_after_revoke = pending.run_owned(vec![
        "--json".to_string(),
        "fs".to_string(),
        "accept".to_string(),
        pending_share_id,
    ]);
    assert_json_error(&accept_after_revoke, "grant_denied");
}

#[cfg(unix)]
#[test]
fn e2e_grant_survives_backing_event_mirror_failure() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    let writer = fixture.agent("writer");

    owner.login("owner");
    owner.create_fs();
    let writer_id = writer.login("writer");

    let events_dir = fixture.remote_root.join(".section/agentfs/events");
    let original_mode = fs::metadata(&events_dir)
        .expect("events dir metadata")
        .permissions()
        .mode();
    let mut readonly = fs::metadata(&events_dir)
        .expect("events dir metadata")
        .permissions();
    readonly.set_mode(0o500);
    fs::set_permissions(&events_dir, readonly).expect("make events dir read-only");

    owner.grant(&writer_id, "writer");

    let mut restored = fs::metadata(&events_dir)
        .expect("events dir metadata")
        .permissions();
    restored.set_mode(original_mode);
    fs::set_permissions(&events_dir, restored).expect("restore events dir permissions");

    assert!(
        !agentfs_event_kinds(&fixture.remote_root)
            .iter()
            .any(|kind| kind == "grant.created"),
        "backing event mirror should be missing grant.created"
    );

    let replay = owner.fs_events(FS_NAME, None);
    let replay_events = replay["events"].as_array().expect("events");
    assert!(
        replay_events.iter().any(|event| {
            event["kind"] == "grant.created"
                && event["data"]["agent_id"] == writer_id
                && event["data"]["role"] == "writer"
        }),
        "service event authority should expose grant.created: {replay_events:?}"
    );
    let mut last_seq = 0;
    for event in replay_events {
        let seq = event["seq"].as_i64().expect("event seq");
        assert!(seq > last_seq, "event seq should be strictly increasing");
        last_seq = seq;
    }

    let writer_share = owner.share(&writer_id);
    let writer_share_id = writer_share["share"]["share_id"]
        .as_str()
        .expect("share id");
    writer.accept(writer_share_id);
    writer.attach();
    writer.write_local("docs/service-grant.txt", "service grant works");
    writer.commit_apply("commit after mirrored event failure");

    assert_eq!(
        fs::read_to_string(fixture.path("docs/service-grant.txt")).expect("read committed file"),
        "service grant works"
    );
}

#[test]
fn e2e_stale_writer_cannot_overwrite_new_truth() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    let writer_a = fixture.agent("writer-a");
    let writer_b = fixture.agent("writer-b");

    owner.login("owner");
    owner.create_fs();

    let writer_a_id = writer_a.login("writer-a");
    owner.grant(&writer_a_id, "writer");
    let writer_a_share = owner.share(&writer_a_id);
    let writer_a_share_id = writer_a_share["share"]["share_id"]
        .as_str()
        .expect("share id");
    writer_a.accept(writer_a_share_id);
    writer_a.attach();

    let writer_b_id = writer_b.login("writer-b");
    owner.grant(&writer_b_id, "writer");
    let writer_b_share = owner.share(&writer_b_id);
    let writer_b_share_id = writer_b_share["share"]["share_id"]
        .as_str()
        .expect("share id");
    writer_b.accept(writer_b_share_id);
    writer_b.attach();

    writer_a.write_local("docs/shared.txt", "from writer a");
    let commit_a = writer_a.commit_apply("writer a update");
    let commit_a_id = commit_a["commit"]["commit_id"]
        .as_str()
        .expect("writer a commit id")
        .to_string();
    assert_eq!(
        fs::read_to_string(fixture.path("docs/shared.txt")).expect("read writer a remote file"),
        "from writer a"
    );

    writer_b.write_local("docs/from-b.txt", "from writer b");
    let marker_path = writer_b.local_root.join(".section/root.json");
    let mut marker = read_json(&marker_path);
    marker["base_commit_id"] = Value::String(commit_a_id.clone());
    write_json(&marker_path, &marker);
    let stale_status = writer_b.fs_status(&writer_b.local_root.to_string_lossy());
    assert_eq!(stale_status["status"]["stale"], true);
    assert!(stale_status["status"]["next_actions"]
        .as_array()
        .expect("stale next actions")
        .iter()
        .any(|action| action == "sync"));
    assert!(!stale_status["status"]["next_actions"]
        .as_array()
        .expect("stale next actions")
        .iter()
        .any(|action| action == "commit"));
    assert_json_error(
        &writer_b.commit_apply_output("writer b update"),
        "stale_base",
    );
    assert!(
        !fixture.path("docs/from-b.txt").exists(),
        "stale writer draft must not materialize"
    );
    let head = read_json(
        fixture
            .remote_root
            .join(".section/agentfs/heads/current.json"),
    );
    assert_eq!(head["commit_id"], commit_a_id);
    assert_eq!(
        fs::read_to_string(fixture.path("docs/shared.txt")).expect("read current truth"),
        "from writer a"
    );
}

#[test]
fn e2e_backing_source_drift_cannot_be_committed_over() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    owner.create_fs();
    owner.attach();

    owner.write_local("docs/drift.txt", "base content");
    let initial_commit = owner.commit_apply("create drift base");
    let initial_commit_id = initial_commit["commit"]["commit_id"]
        .as_str()
        .expect("initial commit id")
        .to_string();
    let accepted_before = owner.fs_events(FS_NAME, None)["events"]
        .as_array()
        .expect("events before drift")
        .iter()
        .filter(|event| event["kind"] == "commit.accepted")
        .count();

    fs::write(fixture.path("docs/drift.txt"), "external drift")
        .expect("write external remote drift");
    owner.write_local("docs/drift.txt", "local update");

    let rejected = owner.commit_apply_output("overwrite external drift");
    let error = assert_json_error(&rejected, "remote_drift");
    assert_eq!(error["error"]["details"]["path"], "docs/drift.txt");
    assert_eq!(
        fs::read_to_string(fixture.path("docs/drift.txt")).expect("read drifted remote file"),
        "external drift"
    );

    let head = read_json(
        fixture
            .remote_root
            .join(".section/agentfs/heads/current.json"),
    );
    assert_eq!(head["commit_id"], initial_commit_id);
    let accepted_after = owner.fs_events(FS_NAME, None)["events"]
        .as_array()
        .expect("events after drift")
        .iter()
        .filter(|event| event["kind"] == "commit.accepted")
        .count();
    assert_eq!(
        accepted_after, accepted_before,
        "remote drift must be rejected before a new accepted commit"
    );
}

#[test]
fn e2e_hardening_rejects_unsafe_backing_source_and_attach_root() {
    let non_empty = AgentFsFixture::new();
    fs::write(non_empty.path("preexisting.txt"), "not imported").expect("write remote file");
    let owner = non_empty.agent("owner");
    owner.login("owner");
    let create = owner.create_fs_output();
    assert!(
        !create.status.success(),
        "fs create unexpectedly accepted non-empty backing source\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&create.stdout),
        String::from_utf8_lossy(&create.stderr)
    );
    assert_eq!(
        owner
            .list_sources()
            .as_array()
            .expect("source list array")
            .len(),
        0
    );
    assert!(
        !non_empty
            .remote_root
            .join(".section/agentfs/fs.json")
            .exists(),
        "rejected fs create must not initialize AgentFS metadata"
    );

    let overlap = AgentFsFixture::new();
    let owner = overlap.agent("owner");
    owner.login("owner");
    owner.create_fs();
    let attach = owner.attach_output(&overlap.remote_root);
    assert!(
        !attach.status.success(),
        "fs attach unexpectedly accepted backing source as working root\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&attach.stdout),
        String::from_utf8_lossy(&attach.stderr)
    );
    assert!(
        !overlap.remote_root.join(".section/root.json").exists(),
        "rejected attach must not write a local root marker into backing source"
    );
}

#[cfg(unix)]
#[test]
fn e2e_create_failure_rolls_back_service_and_local_cache() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");

    let original_mode = fs::metadata(&fixture.remote_root)
        .expect("remote root metadata")
        .permissions()
        .mode();
    let mut readonly = fs::metadata(&fixture.remote_root)
        .expect("remote root metadata")
        .permissions();
    readonly.set_mode(0o500);
    fs::set_permissions(&fixture.remote_root, readonly).expect("make remote root read-only");

    let rejected = owner.create_fs_output();

    let mut restored = fs::metadata(&fixture.remote_root)
        .expect("remote root metadata")
        .permissions();
    restored.set_mode(original_mode);
    fs::set_permissions(&fixture.remote_root, restored).expect("restore remote root permissions");

    assert_json_error(&rejected, "operation_failed");
    assert_eq!(fixture.control_row_count("filesystems"), 0);
    assert_eq!(fixture.control_row_count("grants"), 0);
    assert_eq!(fixture.control_row_count("events"), 0);
    assert_eq!(
        owner
            .list_sources()
            .as_array()
            .expect("source list array")
            .len(),
        0,
        "failed create must remove the local AgentFS source cache"
    );
    assert!(
        !fixture.remote_root.join(".section").exists(),
        "failed create must not leave shared metadata behind"
    );

    owner.create_fs();
    assert_eq!(fixture.control_row_count("filesystems"), 1);
    assert!(fixture.path(".section/agentfs/fs.json").exists());
}

#[test]
fn e2e_attach_canonicalizes_local_root_identity() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    owner.create_fs();

    let parent = fixture.root.join("roots");
    let alias_parent = parent.join("alias");
    let canonical_root = parent.join("canonical-root");
    fs::create_dir_all(&alias_parent).expect("create alias parent");
    fs::create_dir_all(&canonical_root).expect("create canonical root");
    let spelled_root = alias_parent.join("..").join("canonical-root");
    let canonical_root = canonical_root.canonicalize().expect("canonical root");
    let canonical_root_str = canonical_root.to_string_lossy().to_string();

    let attach = owner.attach_to(&spelled_root);
    assert_eq!(
        attach["attach"]["local_root"].as_str(),
        Some(canonical_root_str.as_str())
    );
    let marker = read_json(canonical_root.join(".section/root.json"));
    assert_eq!(
        marker["local_root"].as_str(),
        Some(canonical_root_str.as_str())
    );

    let status = owner.fs_status(&canonical_root.to_string_lossy());
    assert_eq!(
        status["status"]["local_root"].as_str(),
        Some(canonical_root_str.as_str())
    );
    assert_eq!(status["status"]["stale"], false);
}

#[test]
fn e2e_fs_status_reports_corrupt_local_marker() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    owner.create_fs();
    owner.attach();

    let marker_path = owner.local_root.join(".section/root.json");
    fs::write(&marker_path, "{bad marker").expect("corrupt local marker");

    let status = owner.fs_status_output(&owner.local_root.to_string_lossy());
    let error = assert_json_error(&status, "malformed_shared_metadata");
    assert!(
        error["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("local root marker"),
        "status should report the marker parse problem: {error:?}"
    );
}

#[test]
fn e2e_section_directory_is_not_committed_as_user_content() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    owner.create_fs();
    owner.attach();

    fs::write(
        owner.local_root.join(".section/user-note.txt"),
        "must stay local",
    )
    .expect("write local section file");
    owner.write_local("docs/visible.txt", "visible");

    let commit = owner.commit_apply("commit visible file only");
    let paths = commit["commit"]["paths"].as_array().expect("commit paths");
    assert!(
        paths.iter().all(|path| !path["path"]
            .as_str()
            .expect("commit path")
            .starts_with(".section")),
        "commit paths must not include .section content: {paths:?}"
    );
    assert_eq!(
        fs::read_to_string(fixture.path("docs/visible.txt")).expect("remote visible file"),
        "visible"
    );
    assert!(
        !fixture.path(".section/user-note.txt").exists(),
        ".section files must not become shared user content"
    );
}

#[test]
fn e2e_commit_preflight_rejects_empty_message_and_empty_commit() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    owner.create_fs();
    owner.attach();

    owner.write_local("docs/preflight.txt", "draft");
    let empty_message = owner.commit_apply_output("   ");
    assert_json_error(&empty_message, "operation_failed");
    let head = read_json(
        fixture
            .remote_root
            .join(".section/agentfs/heads/current.json"),
    );
    assert!(
        head["commit_id"].is_null(),
        "empty commit message must not advance head"
    );
    assert!(
        !fixture.path("docs/preflight.txt").exists(),
        "empty commit message must not materialize local draft"
    );

    owner.commit_apply("commit preflight draft");
    let clean_status = owner.commit_status();
    assert!(
        clean_status["status"]["dirty_paths"]
            .as_array()
            .expect("dirty paths")
            .is_empty(),
        "working copy should be clean before empty commit check"
    );
    let accepted_before = owner.fs_events(FS_NAME, None)["events"]
        .as_array()
        .expect("events before empty commit")
        .iter()
        .filter(|event| event["kind"] == "commit.accepted")
        .count();

    let empty_commit = owner.commit_apply_output("no dirty paths");
    assert_json_error(&empty_commit, "operation_failed");
    let accepted_after = owner.fs_events(FS_NAME, None)["events"]
        .as_array()
        .expect("events after empty commit")
        .iter()
        .filter(|event| event["kind"] == "commit.accepted")
        .count();
    assert_eq!(
        accepted_after, accepted_before,
        "empty commit must not create a new accepted commit"
    );
}

#[test]
fn e2e_reattach_moves_single_local_root() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    owner.create_fs();
    owner.attach();

    let old_marker = owner.local_root.join(".section/root.json");
    assert!(old_marker.exists(), "initial attach should write marker");

    let second_root = fixture.root.join("owner-second-root");
    owner.attach_to(&second_root);
    assert!(
        !old_marker.exists(),
        "reattach should remove previous local root marker"
    );
    assert!(
        second_root.join(".section/root.json").exists(),
        "reattach should write marker at the new local root"
    );

    assert_json_error(
        &owner.fs_status_output(&owner.local_root.to_string_lossy()),
        "unknown_fs",
    );
    let status = owner.fs_status(&second_root.to_string_lossy());
    let second_root = second_root.canonicalize().expect("canonical second root");
    let second_root_str = second_root.to_string_lossy().to_string();
    assert_eq!(
        status["status"]["local_root"].as_str(),
        Some(second_root_str.as_str())
    );
}

#[test]
fn e2e_rejects_file_dir_type_replacement_before_acceptance() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    owner.create_fs();
    owner.attach();

    owner.write_local("shape", "original file");
    let initial_commit = owner.commit_apply("create file shape");
    let initial_commit_id = initial_commit["commit"]["commit_id"]
        .as_str()
        .expect("initial commit id")
        .to_string();
    assert!(
        fixture.path("shape").is_file(),
        "initial commit should materialize shape as a file"
    );
    let accepted_before = owner.fs_events(FS_NAME, None)["events"]
        .as_array()
        .expect("events before type conflict")
        .iter()
        .filter(|event| event["kind"] == "commit.accepted")
        .count();

    fs::remove_file(owner.local_root.join("shape")).expect("remove local file");
    fs::create_dir(owner.local_root.join("shape")).expect("create local replacement dir");
    fs::write(owner.local_root.join("shape/nested.txt"), "nested").expect("write nested file");

    let rejected = owner.commit_apply_output("replace file with dir");
    let error = assert_json_error(&rejected, "path_type_conflict");
    assert_eq!(error["error"]["details"]["path"], "shape");
    assert_eq!(error["error"]["details"]["local_kind"], "dir");
    assert_eq!(error["error"]["details"]["remote_kind"], "file");

    let head = read_json(
        fixture
            .remote_root
            .join(".section/agentfs/heads/current.json"),
    );
    assert_eq!(head["commit_id"], initial_commit_id);
    assert!(
        fixture.path("shape").is_file(),
        "rejected type replacement must leave remote shape as file"
    );
    assert!(
        !fixture.path("shape/nested.txt").exists(),
        "rejected type replacement must not materialize nested content"
    );
    let accepted_after = owner.fs_events(FS_NAME, None)["events"]
        .as_array()
        .expect("events after type conflict")
        .iter()
        .filter(|event| event["kind"] == "commit.accepted")
        .count();
    assert_eq!(
        accepted_after, accepted_before,
        "type replacement must be rejected before a new accepted commit"
    );
}

#[test]
fn e2e_proposal_approval_flow_keeps_head_governed() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    let contributor = fixture.agent("contributor");
    let manager = fixture.agent("manager");

    owner.login("owner");
    owner.create_fs();
    owner.attach();

    let contributor_id = contributor.login("contributor");
    owner.grant(&contributor_id, "contributor");
    let contributor_share = owner.share(&contributor_id);
    contributor.accept(
        contributor_share["share"]["share_id"]
            .as_str()
            .expect("contributor share id"),
    );
    contributor.attach();

    let manager_id = manager.login("manager");
    owner.grant(&manager_id, "manager");
    let manager_share = owner.share(&manager_id);
    manager.accept(
        manager_share["share"]["share_id"]
            .as_str()
            .expect("manager share id"),
    );

    contributor.write_local("docs/proposal.md", "from proposal");
    let direct_commit = contributor.commit_apply_output("direct contributor commit");
    assert_json_error(&direct_commit, "grant_denied");

    let proposal = contributor.commit_propose("propose docs change");
    let proposal_id = proposal["proposal"]["proposal_id"]
        .as_str()
        .expect("proposal id")
        .to_string();
    assert_eq!(proposal["proposal"]["status"], "proposed");
    assert!(
        !fixture.path("docs/proposal.md").exists(),
        "proposal must not advance shared truth"
    );

    contributor.write_local("docs/stale.md", "stale proposal");
    let stale_proposal = contributor.commit_propose("second proposal before accept");
    let stale_proposal_id = stale_proposal["proposal"]["proposal_id"]
        .as_str()
        .expect("stale proposal id")
        .to_string();

    let manager_accept = manager.commit_accept_output(FS_NAME, &proposal_id);
    assert_json_error(&manager_accept, "grant_denied");

    let accepted = owner.commit_accept(FS_NAME, &proposal_id);
    let accepted_commit_id = accepted["commit"]["commit_id"]
        .as_str()
        .expect("accepted commit id");
    assert_eq!(accepted["commit"]["agent_id"], contributor_id);
    assert_eq!(accepted["commit"]["authorized_by"]["type"], "owner");
    assert_eq!(
        fs::read_to_string(fixture.path("docs/proposal.md")).expect("proposal materialized"),
        "from proposal"
    );

    let stale_accept = owner.commit_accept_output(FS_NAME, &stale_proposal_id);
    assert_json_error(&stale_accept, "stale_base");
    assert!(
        !fixture.path("docs/stale.md").exists(),
        "stale proposal must not materialize"
    );

    let rejected = owner.commit_reject(FS_NAME, &stale_proposal_id);
    assert_eq!(rejected["proposal"]["status"], "rejected");

    let events = owner.fs_events(FS_NAME, None);
    let event_kinds = events["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|event| event["kind"].as_str().expect("kind").to_string())
        .collect::<Vec<_>>();
    assert!(event_kinds.contains(&"commit.proposed".to_string()));
    assert!(event_kinds.contains(&"proposal.accepted".to_string()));
    assert!(event_kinds.contains(&"proposal.rejected".to_string()));
    assert_eq!(
        read_json(
            fixture
                .remote_root
                .join(".section/agentfs/heads/current.json"),
        )["commit_id"],
        accepted_commit_id
    );
}

#[test]
fn e2e_agents_md_protected_paths_are_enforced() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    let writer = fixture.agent("writer");

    owner.login("owner");
    owner.create_fs();
    owner.attach();
    owner.write_local(
        "AGENTS.md",
        r#"---
section:
  protected_paths:
    - docs/locked.md
---

# Rules
"#,
    );
    owner.commit_apply("define protected path");
    owner.write_local("docs/locked.md", "owner allowed");
    owner.commit_apply("owner changes protected path");
    assert_eq!(
        fs::read_to_string(fixture.path("docs/locked.md")).expect("locked path"),
        "owner allowed"
    );

    let writer_id = writer.login("writer");
    owner.grant(&writer_id, "writer");
    let share = owner.share(&writer_id);
    writer.accept(share["share"]["share_id"].as_str().expect("share id"));
    writer.attach();

    writer.write_local("docs/open.md", "allowed");
    writer.commit_apply("change open path");
    assert_eq!(
        fs::read_to_string(fixture.path("docs/open.md")).expect("open path"),
        "allowed"
    );

    writer.write_local("docs/locked.md", "denied");
    let denied = writer.commit_apply_output("change protected path");
    assert_json_error(&denied, "grant_denied");
    assert_eq!(
        fs::read_to_string(fixture.path("docs/locked.md")).expect("locked path after writer deny"),
        "owner allowed",
        "protected path must not be changed by writer"
    );

    writer.write_local(
        "AGENTS.md",
        "---
section:
  protected_paths: [
---

# invalid
",
    );
    let invalid_rules = writer.commit_apply_output("invalid rules");
    assert_json_error(&invalid_rules, "agent_rules_invalid");
    assert!(
        fs::read_to_string(fixture.path("AGENTS.md"))
            .expect("remote AGENTS.md")
            .contains("docs/locked.md"),
        "invalid AGENTS.md must not replace active remote rules"
    );
}

#[test]
fn e2e_path_scoped_grant_restricts_commit_paths() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    let writer = fixture.agent("writer");

    owner.login("owner");
    owner.create_fs();
    owner.attach();

    let writer_id = writer.login("writer");
    let grant = owner.grant_with_scopes(&writer_id, "writer", &["docs/**"]);
    assert_eq!(grant["grant"]["path_scopes"][0], "docs/**");
    let share = owner.share(&writer_id);
    writer.accept(share["share"]["share_id"].as_str().expect("share id"));
    writer.attach();

    writer.write_local("docs/a.md", "allowed");
    let commit = writer.commit_apply("allowed docs change");
    assert_eq!(
        commit["commit"]["authorized_by"]["path_scopes"][0],
        "docs/**"
    );
    assert_eq!(
        commit["commit"]["authorized_by"]["matched_path_scopes"][0],
        "docs/**"
    );
    assert_eq!(
        fs::read_to_string(fixture.path("docs/a.md")).expect("read scoped doc"),
        "allowed"
    );

    writer.write_local("src/a.rs", "denied");
    let denied = writer.commit_apply_output("denied src change");
    assert_json_error(&denied, "path_scope_denied");
    assert!(
        !fixture.path("src/a.rs").exists(),
        "out-of-scope file must not become shared truth"
    );

    writer.write_local("docs/b.md", "also allowed alone");
    let mixed = writer.commit_apply_output("mixed scoped and unscoped change");
    assert_json_error(&mixed, "path_scope_denied");
    assert!(
        !fixture.path("docs/b.md").exists(),
        "mixed commit must be rejected as a whole"
    );
}

#[test]
fn e2e_hooks_v1_run_local_post_materialized_automation() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    let writer = fixture.agent("writer");
    let reader = fixture.agent("reader");
    let shell = test_shell();

    owner.login("owner");
    let create = owner.create_fs();
    let fs_id = create["fs"]["fs_id"].as_str().expect("fs id").to_string();
    owner.attach();

    let writer_id = writer.login("writer");
    owner.grant(&writer_id, "writer");
    let writer_share = owner.share(&writer_id);
    writer.accept(
        writer_share["share"]["share_id"]
            .as_str()
            .expect("writer share id"),
    );
    writer.attach();

    let reader_id = reader.login("reader");
    owner.grant(&reader_id, "reader");
    let reader_share = owner.share(&reader_id);
    reader.accept(
        reader_share["share"]["share_id"]
            .as_str()
            .expect("reader share id"),
    );

    let hook_output_dir = fixture.root.join("hook-output");
    fs::create_dir_all(&hook_output_dir).expect("create hook output dir");
    let hook_script = fixture.root.join("record-hook.sh");
    fs::write(
        &hook_script,
        format!(
            r#"#!/bin/sh
set -eu
mkdir -p {output:?}
cat > {event:?}
printf '%s\n' "$SECTION_FS_ID" > {fs_id:?}
printf '%s\n' "$SECTION_HOOK_ID" > {hook_id:?}
printf '%s\n' "$SECTION_EVENT_KIND" > {kind:?}
printf '%s\n' "$SECTION_LOCAL_ROOT" > {local_root:?}
printf '%s\n' "${{SECTION_TEST_PARENT_SECRET-}}" > {parent_secret:?}
"#,
            output = hook_output_dir,
            event = hook_output_dir.join("event.json"),
            fs_id = hook_output_dir.join("fs_id.txt"),
            hook_id = hook_output_dir.join("hook_id.txt"),
            kind = hook_output_dir.join("kind.txt"),
            local_root = hook_output_dir.join("local_root.txt"),
            parent_secret = hook_output_dir.join("parent_secret.txt"),
        ),
    )
    .expect("write hook script");

    let hook = owner.hooks_add(
        FS_NAME,
        "record",
        &[
            shell.as_str(),
            hook_script.to_str().expect("utf8 hook script"),
        ],
    );
    let hook_id = hook["hook"]["hook_id"]
        .as_str()
        .expect("hook id")
        .to_string();
    assert_eq!(hook["hook"]["event"], "commit.materialized");
    assert_eq!(
        hook["hook"]["created_by_agent_id"],
        create["fs"]["owner_agent_id"]
    );

    let writer_hooks = writer.hooks_list(FS_NAME);
    assert!(writer_hooks["hooks"]
        .as_array()
        .expect("writer hooks")
        .iter()
        .any(|hook| hook["hook_id"] == hook_id));

    let denied = reader.hooks_add_output(FS_NAME, "reader-hook", &[shell.as_str(), "-c", "true"]);
    assert_json_error(&denied, "grant_denied");

    let removable = owner.hooks_add(FS_NAME, "remove-me", &[shell.as_str(), "-c", "true"]);
    let removable_hook_id = removable["hook"]["hook_id"]
        .as_str()
        .expect("removable hook id");
    let removed = owner.hooks_remove(FS_NAME, removable_hook_id);
    assert_eq!(removed["hook"]["hook_id"], removable_hook_id);
    let hooks_after_remove = owner.hooks_list(FS_NAME);
    assert!(!hooks_after_remove["hooks"]
        .as_array()
        .expect("hooks after remove")
        .iter()
        .any(|hook| hook["hook_id"] == removable_hook_id));

    writer.write_local("docs/hooked.txt", "hooked content");
    let commit = writer.commit_apply_with_env(
        "commit with hook",
        &[("SECTION_TEST_PARENT_SECRET", "secret")],
    );
    let commit_id = commit["commit"]["commit_id"].as_str().expect("commit id");
    assert_eq!(commit["commit"]["materialization_state"], "materialized");

    let event = read_json(hook_output_dir.join("event.json"));
    assert_eq!(event["kind"], "commit.materialized");
    assert_eq!(event["fs_id"], fs_id);
    assert_eq!(event["subject_id"], commit_id);
    assert_eq!(event["actor_agent_id"], writer_id);
    assert_eq!(
        fs::read_to_string(hook_output_dir.join("fs_id.txt")).expect("hook fs id"),
        format!("{fs_id}\n")
    );
    assert_eq!(
        fs::read_to_string(hook_output_dir.join("hook_id.txt")).expect("hook id file"),
        format!("{hook_id}\n")
    );
    assert_eq!(
        fs::read_to_string(hook_output_dir.join("kind.txt")).expect("hook kind"),
        "commit.materialized\n"
    );
    assert_eq!(
        fs::read_to_string(hook_output_dir.join("local_root.txt")).expect("hook root"),
        format!("{}\n", writer.local_root.display())
    );
    assert_eq!(
        fs::read_to_string(hook_output_dir.join("parent_secret.txt")).expect("hook parent secret"),
        "\n",
        "hook must not inherit the section process environment"
    );

    let runs = writer.hook_runs(&fs_id);
    assert_eq!(runs.len(), 1, "writer should record one hook run: {runs:?}");
    assert_eq!(runs[0]["hook_id"], hook_id);
    assert_eq!(runs[0]["event_id"], event["event_id"]);
    assert_eq!(runs[0]["status"], "success");
    assert_eq!(runs[0]["exit_code"], 0);
    assert!(
        reader.hook_runs(&fs_id).is_empty(),
        "agent without attached local root must not record hook runs"
    );

    let failing = owner.hooks_add(
        FS_NAME,
        "fail",
        &[shell.as_str(), "-c", "echo fail >&2; exit 7"],
    );
    let failing_hook_id = failing["hook"]["hook_id"]
        .as_str()
        .expect("failing hook id");
    writer.write_local("docs/hooked-again.txt", "still commits");
    let second_commit = writer.commit_apply("commit with failing hook");
    assert_eq!(
        second_commit["commit"]["materialization_state"],
        "materialized"
    );
    assert_eq!(
        fs::read_to_string(fixture.path("docs/hooked-again.txt")).expect("remote hooked again"),
        "still commits"
    );

    let runs = writer.hook_runs(&fs_id);
    assert!(
        runs.iter().any(|run| run["hook_id"] == failing_hook_id
            && run["status"] == "failed"
            && run["exit_code"] == 7
            && run["stderr_tail"]
                .as_str()
                .expect("stderr")
                .contains("fail")),
        "failing hook should be recorded locally without failing commit: {runs:?}"
    );
}

#[test]
fn e2e_hooks_management_uses_control_service_without_backing_source_read() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    let shell = test_shell();

    owner.login("owner");
    owner.create_fs();
    let head_path = fixture
        .remote_root
        .join(".section/agentfs/heads/current.json");
    fs::write(&head_path, "{not valid json").expect("corrupt backing source head");

    let hook = owner.hooks_add(FS_NAME, "control-only", &[shell.as_str(), "-c", "true"]);
    let hook_id = hook["hook"]["hook_id"]
        .as_str()
        .expect("control-only hook id");
    let hooks = owner.hooks_list(FS_NAME);
    assert!(
        hooks["hooks"]
            .as_array()
            .expect("hooks")
            .iter()
            .any(|hook| hook["hook_id"] == hook_id),
        "hook management should resolve through Control Service: {hooks:?}"
    );
    let removed = owner.hooks_remove(FS_NAME, hook_id);
    assert_eq!(removed["hook"]["hook_id"], hook_id);
}

#[test]
fn e2e_non_empty_directory_delete_materializes_cleanly() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    owner.create_fs();
    owner.attach();

    owner.write_local("docs/tree/a.txt", "a");
    owner.write_local("docs/tree/nested/b.txt", "b");
    owner.commit_apply("create nested tree");
    assert!(fixture.path("docs/tree/a.txt").is_file());
    assert!(fixture.path("docs/tree/nested/b.txt").is_file());

    fs::remove_dir_all(owner.local_root.join("docs/tree")).expect("remove local tree");
    let delete_commit = owner.commit_apply("delete nested tree");
    assert_eq!(
        delete_commit["commit"]["materialization_state"],
        "materialized"
    );
    assert!(
        !fixture.path("docs/tree").exists(),
        "remote subtree should be removed by the delete commit"
    );
    assert!(delete_commit["commit"]["paths"]
        .as_array()
        .expect("delete commit paths")
        .iter()
        .any(|path| path["path"]
            .as_str()
            .expect("path")
            .starts_with("docs/tree")
            && path["op"] == "delete"));

    let status = owner.commit_status();
    assert!(
        status["status"]["dirty_paths"]
            .as_array()
            .expect("dirty paths")
            .is_empty(),
        "delete commit should leave the working copy clean: {status:?}"
    );
}

#[cfg(unix)]
#[test]
fn e2e_materialization_failure_emits_fs_error_event() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    owner.create_fs();
    owner.attach();

    owner.write_local("docs/base.txt", "base");
    let base_commit = owner.commit_apply("create docs base");
    let base_commit_id = base_commit["commit"]["commit_id"]
        .as_str()
        .expect("base commit id")
        .to_string();

    let docs_path = fixture.path("docs");
    let original_mode = fs::metadata(&docs_path)
        .expect("docs metadata")
        .permissions()
        .mode();
    let mut readonly = fs::metadata(&docs_path)
        .expect("docs metadata")
        .permissions();
    readonly.set_mode(0o500);
    fs::set_permissions(&docs_path, readonly).expect("make remote docs read-only");

    owner.write_local("docs/fail.txt", "cannot materialize");
    let rejected = owner.commit_apply_output("fail materialization");

    let mut restored = fs::metadata(&docs_path)
        .expect("docs metadata")
        .permissions();
    restored.set_mode(original_mode);
    fs::set_permissions(&docs_path, restored).expect("restore remote docs permissions");

    let error = assert_json_error(&rejected, "materialization_failed");
    assert_eq!(error["error"]["retryable"], true);
    assert!(
        !fixture.path("docs/fail.txt").exists(),
        "failed materialization must not write the blocked file"
    );

    let head = read_json(
        fixture
            .remote_root
            .join(".section/agentfs/heads/current.json"),
    );
    let failed_commit_id = head["commit_id"]
        .as_str()
        .expect("failed head commit id")
        .to_string();
    assert_ne!(failed_commit_id, base_commit_id);
    let failed_commit = read_json(
        fixture
            .remote_root
            .join(".section/agentfs/commits")
            .join(format!("{failed_commit_id}.json")),
    );
    assert_eq!(
        failed_commit["materialization_state"],
        "failed_to_materialize"
    );

    let events = owner.fs_events(FS_NAME, None);
    let replay_events = events["events"].as_array().expect("events");
    assert!(
        replay_events.iter().any(|event| {
            event["kind"] == "commit.materialization_failed"
                && event["subject_id"] == failed_commit_id
        }),
        "commit failure event must be replayable: {replay_events:?}"
    );
    let fs_error = replay_events
        .iter()
        .find(|event| event["kind"] == "fs.error" && event["subject_id"] == failed_commit_id)
        .expect("fs.error event");
    assert_eq!(fs_error["data"]["code"], "materialization_failed");
    assert_eq!(fs_error["data"]["commit_id"], failed_commit_id);
    assert_eq!(
        fs_error["data"]["materialization_state"],
        "failed_to_materialize"
    );
    assert!(fs_error["data"]["paths"]
        .as_array()
        .expect("fs.error paths")
        .iter()
        .any(|path| path["path"] == "docs/fail.txt" && path["op"] == "create"));
}

#[test]
fn e2e_event_write_failure_does_not_advance_commit_head() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    owner.create_fs();
    owner.attach();

    let events_path = fixture.remote_root.join(".section/agentfs/events");
    fs::remove_dir_all(&events_path).expect("remove events directory");
    fs::write(&events_path, "not a directory").expect("block event writes");

    owner.write_local("blocked.txt", "must stay local");
    let rejected = owner.commit_apply_output("must not advance without event");
    assert_json_error(&rejected, "operation_failed");

    let head = read_json(
        fixture
            .remote_root
            .join(".section/agentfs/heads/current.json"),
    );
    assert!(
        head["commit_id"].is_null(),
        "head must not advance when commit.accepted event cannot be written"
    );
    let commits_path = fixture.remote_root.join(".section/agentfs/commits");
    let commit_count = if commits_path.exists() {
        fs::read_dir(&commits_path)
            .expect("read commits directory")
            .count()
    } else {
        0
    };
    assert_eq!(
        commit_count, 0,
        "event log preflight failure must not write a commit record"
    );
    assert!(
        !fixture.path("blocked.txt").exists(),
        "event write failure must not materialize user content"
    );
}

#[test]
fn e2e_metadata_head_lock_blocks_commit_until_released() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    let owner_id = owner.login("owner");
    let create = owner.create_fs();
    let fs_id = create["fs"]["fs_id"].as_str().expect("fs id").to_string();
    owner.attach();

    let lock_token = "lck_11111111111111111111111111111111";
    let lock_dir = fixture.remote_root.join(".section/agentfs/locks/head");
    fs::create_dir_all(&lock_dir).expect("create lock dir");
    let lock_path = lock_dir.join(format!("{lock_token}.json"));
    write_json(
        &lock_path,
        &serde_json::json!({
            "schema_version": 1,
            "fs_id": fs_id,
            "lock_token": lock_token,
            "owner_agent_id": owner_id,
            "created_at_ms": 1,
            "expires_at_ms": 9999999999999_i64,
        }),
    );

    owner.write_local("docs/locked.txt", "blocked by lock");
    let rejected = owner.commit_apply_output("blocked by active metadata lock");
    assert_json_error(&rejected, "metadata_write_conflict");
    assert!(
        !fixture.path("docs/locked.txt").exists(),
        "locked commit must not materialize user content"
    );

    fs::remove_file(&lock_path).expect("remove lock");
    let commit = owner.commit_apply("commit after lock release");
    let commit_id = commit["commit"]["commit_id"]
        .as_str()
        .expect("commit id")
        .to_string();
    let head = read_json(
        fixture
            .remote_root
            .join(".section/agentfs/heads/current.json"),
    );
    assert_eq!(head["commit_id"], commit_id);
    assert_eq!(
        fs::read_to_string(fixture.path("docs/locked.txt")).expect("read committed file"),
        "blocked by lock"
    );
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn e2e_rejects_non_utf8_commit_paths() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    owner.create_fs();
    owner.attach();

    let accepted_before = owner.fs_events(FS_NAME, None)["events"]
        .as_array()
        .expect("events before non-utf8")
        .iter()
        .filter(|event| event["kind"] == "commit.accepted")
        .count();
    let bad_name = OsString::from_vec(vec![b'b', b'a', b'd', b'-', 0xff, b'.', b't', b'x', b't']);
    fs::write(owner.local_root.join(bad_name), "must stay local").expect("write non-utf8 file");

    let rejected = owner.commit_apply_output("must reject non-utf8");
    let error = assert_json_error(&rejected, "non_utf8_path");
    assert!(error["error"]["details"]["local_path"]
        .as_str()
        .expect("local path detail")
        .contains("bad-"));

    let head = read_json(
        fixture
            .remote_root
            .join(".section/agentfs/heads/current.json"),
    );
    assert!(head["commit_id"].is_null());
    let accepted_after = owner.fs_events(FS_NAME, None)["events"]
        .as_array()
        .expect("events after non-utf8")
        .iter()
        .filter(|event| event["kind"] == "commit.accepted")
        .count();
    assert_eq!(
        accepted_after, accepted_before,
        "non-utf8 path rejection must not accept a commit"
    );
}

#[cfg(unix)]
#[test]
fn e2e_commit_success_survives_local_marker_update_failure() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    owner.create_fs();
    owner.attach();

    let marker_path = owner.local_root.join(".section/root.json");
    let mut readonly = fs::metadata(&marker_path)
        .expect("marker metadata")
        .permissions();
    readonly.set_mode(0o400);
    fs::set_permissions(&marker_path, readonly).expect("make marker read-only");

    owner.write_local("marker-warning.txt", "committed despite marker warning");
    let commit = owner.commit_apply("commit despite marker warning");
    let commit_id = commit["commit"]["commit_id"]
        .as_str()
        .expect("commit id")
        .to_string();
    let warnings = commit["warnings"].as_array().expect("commit warnings");
    assert!(
        warnings.iter().any(|warning| warning
            .as_str()
            .expect("warning string")
            .contains("local root marker update failed")),
        "commit should report marker update warning: {warnings:?}"
    );

    assert_eq!(
        fs::read_to_string(fixture.path("marker-warning.txt")).expect("remote committed file"),
        "committed despite marker warning"
    );
    let head = read_json(
        fixture
            .remote_root
            .join(".section/agentfs/heads/current.json"),
    );
    assert_eq!(head["commit_id"], commit_id);

    let status = owner.fs_status(&owner.local_root.to_string_lossy());
    assert_eq!(status["status"]["base_commit_id"], commit_id);
    assert_eq!(status["status"]["stale"], false);

    let mut writable = fs::metadata(&marker_path)
        .expect("marker metadata")
        .permissions();
    writable.set_mode(0o600);
    fs::set_permissions(&marker_path, writable).expect("restore marker permissions");
}

#[cfg(unix)]
#[test]
fn e2e_hardening_rejects_symlink_commit_paths() {
    let fixture = AgentFsFixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    owner.create_fs();
    owner.attach();

    let outside_file = fixture.root.join("outside-secret.txt");
    fs::write(&outside_file, "must not leak").expect("write outside file");
    std::os::unix::fs::symlink(&outside_file, owner.local_root.join("leak.txt"))
        .expect("create symlink");

    let commit = owner.commit_apply_output("must reject symlink");
    assert!(
        !commit.status.success(),
        "commit unexpectedly followed a symlink\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&commit.stdout),
        String::from_utf8_lossy(&commit.stderr)
    );
    assert!(
        !fixture.path("leak.txt").exists(),
        "symlink target must not materialize as remote content"
    );
}
