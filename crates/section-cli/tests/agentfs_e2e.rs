mod support;

use crate::support::{assert_success, run_section, write_agentfs_config};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

const FS_NAME: &str = "project";
const SOURCE_PROFILE: &str = "test-profile";

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
            local_root: self.root.join(format!("{name}-root")),
        }
    }

    fn path(&self, relative_path: &str) -> PathBuf {
        self.remote_root.join(relative_path)
    }
}

struct AgentFsActor {
    config_path: PathBuf,
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
        self.json_owned(vec![
            "--json".to_string(),
            "fs".to_string(),
            "grant".to_string(),
            FS_NAME.to_string(),
            agent_id.to_string(),
            "--role".to_string(),
            role.to_string(),
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

    fn commit_apply(&self, message: &str) -> Value {
        let output = self.commit_apply_output(message);
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

    fn fs_events(&self, fs_ref: &str, after: Option<&str>) -> Value {
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
        self.json_owned(args)
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

fn assert_json_error(output: &Output, code: &str) {
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let error: Value = serde_json::from_slice(&output.stdout).expect("error json");
    assert_eq!(error["error"]["code"], code);
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
    assert!(
        !serde_json::to_string(&accept)
            .expect("accept json string")
            .contains(fixture.remote_root.to_str().expect("utf8 remote root")),
        "accept JSON must not expose backing source paths"
    );

    let attach = writer.attach();
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

    let owner_fresh_root = fixture.root.join("owner-fresh-root");
    owner.attach_to(&owner_fresh_root);
    assert_eq!(
        fs::read_to_string(owner_fresh_root.join("docs/note.txt"))
            .expect("owner reads accepted writer content"),
        "hello from writer"
    );
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

    let writer_id = writer.login("writer");
    owner.grant(&writer_id, "writer");
    let writer_share = owner.share(&writer_id);
    let writer_share_id = writer_share["share"]["share_id"]
        .as_str()
        .expect("share id");
    writer.accept(writer_share_id);
    writer.attach();
    owner.grant(&writer_id, "reader");
    writer.write_local("writer-after-downgrade.txt", "writer local draft");
    assert_json_error(
        &writer.commit_apply_output("writer after downgrade"),
        "grant_denied",
    );
    assert!(
        !fixture.path("writer-after-downgrade.txt").exists(),
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
    manager.attach();
    manager.write_local("manager-draft.txt", "manager local draft");
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
