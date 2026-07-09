mod support;

use crate::support::{
    assert_success, run_section, write_agentfs_config, write_agentfs_endpoint_config, write_config,
};
use serde_json::Value;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::thread;

const FS_NAME: &str = "project";
const SOURCE_PROFILE: &str = "test-profile";

fn test_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|shell| shell.starts_with('/') && Path::new(shell).exists())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

struct Fixture {
    _temp_dir: tempfile::TempDir,
    root: PathBuf,
    remote_root: PathBuf,
    control_service_path: PathBuf,
}

impl Fixture {
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

    fn agent(&self, name: &str) -> Actor {
        self.agent_with_remote(name, &self.remote_root)
    }

    fn agent_with_remote(&self, name: &str, remote_root: &Path) -> Actor {
        let data_dir = self.root.join(format!("{name}-data"));
        let config_path = self.root.join(format!("{name}.toml"));
        write_agentfs_config(
            &config_path,
            &data_dir,
            &self.control_service_path,
            SOURCE_PROFILE,
            remote_root,
        );
        Actor {
            config_path,
            data_dir,
            local_root: self.root.join(format!("{name}-root")),
        }
    }

    fn agent_with_endpoint(&self, name: &str, endpoint: &str) -> Actor {
        let data_dir = self.root.join(format!("{name}-data"));
        let config_path = self.root.join(format!("{name}.toml"));
        write_agentfs_endpoint_config(&config_path, &data_dir, endpoint);
        Actor {
            config_path,
            data_dir,
            local_root: self.root.join(format!("{name}-root")),
        }
    }

    fn remote_path(&self, path: &str) -> PathBuf {
        self.remote_root.join(path)
    }
}

fn start_control_service_server(config_path: PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind control service test listener");
    let addr = listener.local_addr().expect("control service local addr");
    listener
        .set_nonblocking(true)
        .expect("set listener nonblocking");
    thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("control service runtime");
        runtime
            .block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener)
                    .expect("tokio control service listener");
                sectiond::serve_control_service_listener(Some(&config_path), listener).await
            })
            .expect("control service server");
    });
    format!("http://{addr}")
}

struct Actor {
    config_path: PathBuf,
    data_dir: PathBuf,
    local_root: PathBuf,
}

impl Actor {
    fn login_output(&self, name: &str) -> Output {
        self.run_owned(vec![
            "--json".to_string(),
            "agent".to_string(),
            "login".to_string(),
            name.to_string(),
        ])
    }

    fn login(&self, name: &str) -> Value {
        let output = self.login_output(name);
        assert_success(&output, "agent login");
        serde_json::from_slice(&output.stdout).expect("login json")
    }

    fn identify(&self) -> Value {
        self.json(&["--json", "agent", "identify"], "agent identify")
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

    fn list_fs(&self) -> Value {
        self.json(&["--json", "fs", "list"], "fs list")
    }

    fn source_list(&self) -> Value {
        self.json(&["--json", "source", "list"], "source list")
    }

    fn runtime_status(&self) -> Value {
        self.json(&["--json", "status"], "section status")
    }

    fn source_sync_output(&self, source_name: &str) -> Output {
        self.run_owned(vec![
            "--json".to_string(),
            "source".to_string(),
            "sync".to_string(),
            source_name.to_string(),
        ])
    }

    fn source_bind_output(&self, source_name: &str, local_root: &Path) -> Output {
        self.run_owned(vec![
            "--json".to_string(),
            "source".to_string(),
            "bind".to_string(),
            source_name.to_string(),
            local_root.to_str().expect("utf8 local root").to_string(),
        ])
    }

    fn source_remove_output(&self, source_name: &str) -> Output {
        self.run_owned(vec![
            "--json".to_string(),
            "source".to_string(),
            "remove".to_string(),
            source_name.to_string(),
        ])
    }

    fn status(&self, fs: &str) -> Value {
        self.json_owned(vec![
            "--json".to_string(),
            "fs".to_string(),
            "status".to_string(),
            fs.to_string(),
        ])
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

    fn available_output(&self) -> Output {
        self.run_owned(vec![
            "--json".to_string(),
            "fs".to_string(),
            "available".to_string(),
        ])
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

    fn hooks_add(&self, fs_ref: &str, name: &str, command: &[&str]) -> Value {
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
        self.json_owned(args)
    }

    fn hooks_list(&self, fs_ref: &str) -> Value {
        self.json_owned(vec![
            "--json".to_string(),
            "hooks".to_string(),
            "list".to_string(),
            fs_ref.to_string(),
        ])
    }

    fn write_output(&self, path: &str) -> Output {
        self.run_owned(vec![
            "--json".to_string(),
            "write".to_string(),
            path.to_string(),
        ])
    }

    fn path_inspect_output(&self) -> Output {
        self.run_owned(vec![
            "--json".to_string(),
            "path".to_string(),
            "inspect".to_string(),
            self.local_root
                .to_str()
                .expect("utf8 local root")
                .to_string(),
        ])
    }

    fn append_config_source(&self, source_name: &str, source_root: &Path) {
        let mut config = fs::read_to_string(&self.config_path).expect("read config");
        config.push_str(&format!(
            "\n[sources.{source_name:?}]\nprovider = \"fs\"\n\n[sources.{source_name:?}.options]\nroot = {:?}\n",
            source_root.to_string_lossy(),
        ));
        fs::write(&self.config_path, config).expect("append config source");
    }

    fn commit_apply(&self, message: &str) -> Value {
        let output = self.commit_apply_output(message);
        assert_success(&output, "commit apply");
        serde_json::from_slice(&output.stdout).expect("commit apply json")
    }

    fn write_local(&self, path: &str, content: &str) {
        let path = self.local_root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create local parent");
        }
        fs::write(path, content).expect("write local file");
    }

    fn run_owned(&self, args: Vec<String>) -> Output {
        let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        run_section(&self.config_path, &refs)
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
    serde_json::from_slice(&fs::read(path).expect("read json")).expect("json")
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

fn share_and_accept(owner: &Actor, target: &Actor, target_id: &str) {
    let share = owner.share(target_id);
    let share_id = share["share"]["share_id"].as_str().expect("share id");
    let available = target.available();
    assert!(available["available"]
        .as_array()
        .expect("available shares")
        .iter()
        .any(|entry| entry["share"]["share_id"] == share_id));
    target.accept(share_id);
}

fn login_agent_id(login: &Value) -> String {
    login["agent"]["agent_id"]
        .as_str()
        .expect("agent id")
        .to_string()
}

fn assert_json_output_omits(output: &Output, forbidden: &[&str], context: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    for value in forbidden {
        assert!(
            !stdout.contains(value),
            "{context} JSON leaked {value:?}\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn http_control_service_shares_without_client_source_profile_or_keys() {
    let fixture = Fixture::new();
    let shell = test_shell();
    let server_config = fixture.root.join("control-server.toml");
    let server_data = fixture.root.join("control-server-data");
    write_agentfs_config(
        &server_config,
        &server_data,
        &fixture.control_service_path,
        SOURCE_PROFILE,
        &fixture.remote_root,
    );
    let endpoint = start_control_service_server(server_config);
    let owner = fixture.agent_with_endpoint("owner", &endpoint);
    let writer = fixture.agent_with_endpoint("writer", &endpoint);

    let owner_config = fs::read_to_string(&owner.config_path).expect("read owner config");
    let writer_config = fs::read_to_string(&writer.config_path).expect("read writer config");
    let remote_root = fixture.remote_root.to_string_lossy();
    assert!(!owner_config.contains(SOURCE_PROFILE));
    assert!(!writer_config.contains(SOURCE_PROFILE));
    assert!(!owner_config.contains(remote_root.as_ref()));
    assert!(!writer_config.contains(remote_root.as_ref()));

    owner.login("owner");
    let create = owner.create_fs();
    assert_eq!(create["fs"]["name"], FS_NAME);
    let fs_id = create["fs"]["fs_id"].as_str().expect("fs id").to_string();
    owner.attach();

    let writer_id = login_agent_id(&writer.login("writer"));
    let writer_store = section_provider::ProviderStore::open(&writer.data_dir)
        .expect("open writer provider store");
    let original_writer_identity = writer_store
        .get_agent_identity()
        .expect("read writer identity")
        .expect("writer identity");
    let mut tampered_writer_identity = original_writer_identity.clone();
    tampered_writer_identity.auth_token = "auth_invalid".to_string();
    writer_store
        .cache_agent_identity(&tampered_writer_identity)
        .expect("tamper writer identity");
    assert_json_error(&writer.available_output(), "unknown_agent");
    writer_store
        .cache_agent_identity(&original_writer_identity)
        .expect("restore writer identity");

    owner.grant(&writer_id, "writer");
    let share = owner.share(&writer_id);
    let share_id = share["share"]["share_id"].as_str().expect("share id");
    assert!(writer.available()["available"]
        .as_array()
        .expect("available shares")
        .iter()
        .any(|available| available["share"]["share_id"] == share_id));
    let accept = writer.accept(share_id);
    assert_eq!(accept["fs"]["name"], FS_NAME);
    assert!(
        accept["credential_binding"]["credential_binding_id"]
            .as_str()
            .expect("credential binding id")
            .starts_with("cred_"),
        "accept must return a service-issued credential binding"
    );
    writer.attach();

    let hook_output_dir = fixture.root.join("http-hook-output");
    fs::create_dir_all(&hook_output_dir).expect("create hook output dir");
    let hook_script = fixture.root.join("http-record-hook.sh");
    fs::write(
        &hook_script,
        format!(
            r#"#!/bin/sh
set -eu
mkdir -p {output:?}
cat > {event:?}
"#,
            output = hook_output_dir,
            event = hook_output_dir.join("event.json"),
        ),
    )
    .expect("write http hook script");
    let hook = owner.hooks_add(
        FS_NAME,
        "http-record",
        &[
            shell.as_str(),
            hook_script.to_str().expect("utf8 hook script"),
        ],
    );
    let hook_id = hook["hook"]["hook_id"]
        .as_str()
        .expect("hook id")
        .to_string();
    assert!(writer.hooks_list(FS_NAME)["hooks"]
        .as_array()
        .expect("http hooks")
        .iter()
        .any(|hook| hook["hook_id"] == hook_id));

    writer.write_local("docs/remote-service.txt", "from http control service");
    let commit = writer.commit_apply("commit through http control service");
    let commit_id = commit["commit"]["commit_id"]
        .as_str()
        .expect("http commit id");
    assert_eq!(
        fs::read_to_string(fixture.remote_path("docs/remote-service.txt"))
            .expect("read remote committed file"),
        "from http control service"
    );
    let event = read_json(hook_output_dir.join("event.json"));
    assert_eq!(event["kind"], "commit.materialized");
    assert_eq!(event["fs_id"], fs_id);
    assert_eq!(event["subject_id"], commit_id);
    let writer_store =
        section_provider::ProviderStore::open(&writer.data_dir).expect("open writer store");
    let runs = writer_store
        .list_agentfs_hook_runs(&fs_id)
        .expect("list http hook runs");
    assert!(
        runs.iter()
            .any(|run| run.hook_id == hook_id && run.status == "success"),
        "http hook run should be stored locally: {runs:?}"
    );
}

#[test]
fn agent_login_and_fs_create_write_service_backed_metadata() {
    let fixture = Fixture::new();
    let owner = fixture.agent("owner");

    let login = owner.login("owner");
    let owner_id = login["agent"]["agent_id"].as_str().expect("owner id");
    let installation_id = login["installation"]["installation_id"]
        .as_str()
        .expect("installation id");
    assert!(owner_id.starts_with("agt_"));
    assert!(installation_id.starts_with("ins_"));
    assert!(
        !serde_json::to_string(&login)
            .expect("login json string")
            .contains("auth_"),
        "login JSON must not expose the local auth token"
    );
    assert_eq!(
        owner.identify()["installation"]["installation_id"],
        installation_id
    );

    let create = owner.create_fs();
    let fs_id = create["fs"]["fs_id"].as_str().expect("fs id");
    let source_profile_id = create["fs"]["source_profile_id"]
        .as_str()
        .expect("source profile id");
    assert!(fs_id.starts_with("fs_"));
    assert!(source_profile_id.starts_with("srcp_"));
    assert_eq!(create["fs"]["owner_agent_id"], owner_id);
    assert_eq!(create["fs"]["source_name"], fs_id);

    let fs_json = read_json(fixture.remote_root.join(".section/agentfs/fs.json"));
    assert_eq!(fs_json["fs_id"], fs_id);
    assert_eq!(fs_json["source_profile_id"], source_profile_id);

    let head_json = read_json(
        fixture
            .remote_root
            .join(".section/agentfs/heads/current.json"),
    );
    assert_eq!(head_json["fs_id"], fs_id);
    assert!(head_json["commit_id"].is_null());

    assert_eq!(
        fs::read_dir(fixture.remote_root.join(".section/agentfs/grants"))
            .expect("grants dir")
            .count(),
        1
    );
    assert!(agentfs_event_kinds(&fixture.remote_root).contains(&"fs.created".to_string()));

    let list = owner.list_fs();
    let filesystems = list.as_array().expect("fs list array");
    assert_eq!(filesystems.len(), 1);
    assert_eq!(filesystems[0]["fs_id"], fs_id);

    let status = owner.status(FS_NAME);
    assert_eq!(status["status"]["role"], "owner");
    assert_eq!(status["status"]["head"]["commit_id"], Value::Null);
}

#[test]
fn fresh_data_dir_cannot_claim_existing_agent_name_without_token() {
    let fixture = Fixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    owner.create_fs();

    let imposter = fixture.agent("imposter");
    let login = imposter.login_output("owner");
    assert!(
        !login.status.success(),
        "fresh data dir unexpectedly logged in as existing owner\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&login.stdout),
        String::from_utf8_lossy(&login.stderr)
    );
}

#[test]
fn source_json_outputs_omit_raw_options_roots_and_secrets() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().join("data");
    let config_path = temp_dir.path().join("config.toml");
    let source_root = temp_dir.path().join("remote-source-root");
    let local_root = temp_dir.path().join("local-bind-root");
    fs::create_dir_all(&source_root).expect("create source root");
    write_config(&config_path, &data_dir);

    let source_root_string = source_root.to_str().expect("utf8 source root").to_string();
    let secret = "super-secret-token";
    let root_opt = format!("root={source_root_string}");
    let secret_opt = format!("secret_access_key={secret}");

    let add = run_section(
        &config_path,
        &[
            "--json",
            "source",
            "add",
            "leaky",
            "--provider",
            "fs",
            "--opt",
            &root_opt,
            "--opt",
            &secret_opt,
        ],
    );
    assert_success(&add, "source add json");
    assert_json_output_omits(
        &add,
        &[
            &source_root_string,
            secret,
            "secret_access_key",
            "\"options\"",
        ],
        "source add",
    );
    let add_json: Value = serde_json::from_slice(&add.stdout).expect("add json");
    assert!(add_json["source"]["options"].is_null());

    let bind = run_section(
        &config_path,
        &[
            "--json",
            "source",
            "bind",
            "leaky",
            local_root.to_str().expect("utf8 local root"),
        ],
    );
    assert_success(&bind, "source bind json");
    assert_json_output_omits(
        &bind,
        &[
            &source_root_string,
            secret,
            "secret_access_key",
            "\"options\"",
        ],
        "source bind",
    );
    let bind_json: Value = serde_json::from_slice(&bind.stdout).expect("bind json");
    assert!(bind_json["source"]["options"].is_null());

    let list = run_section(&config_path, &["--json", "source", "list"]);
    assert_success(&list, "source list json");
    assert_json_output_omits(
        &list,
        &[
            &source_root_string,
            secret,
            "secret_access_key",
            "\"options\"",
        ],
        "source list",
    );
    let list_json: Value = serde_json::from_slice(&list.stdout).expect("list json");
    assert!(list_json[0]["options"].is_null());
}

#[test]
fn writer_share_accept_attach_commit_and_owner_observes_truth() {
    let fixture = Fixture::new();
    let owner = fixture.agent("owner");
    let writer = fixture.agent("writer");

    let owner_id = owner.login("owner")["agent"]["agent_id"]
        .as_str()
        .expect("owner id")
        .to_string();
    let create = owner.create_fs();
    let fs_id = create["fs"]["fs_id"].as_str().expect("fs id").to_string();
    let source_profile_id = create["fs"]["source_profile_id"]
        .as_str()
        .expect("source profile id")
        .to_string();
    owner.attach();

    let writer_login = writer.login("writer");
    let writer_id = login_agent_id(&writer_login);
    let writer_installation_id = writer_login["installation"]["installation_id"]
        .as_str()
        .expect("writer installation id")
        .to_string();
    let grant = owner.grant(&writer_id, "writer");
    let grant_id = grant["grant"]["grant_id"]
        .as_str()
        .expect("grant id")
        .to_string();
    share_and_accept(&owner, &writer, &writer_id);

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
    let source_list = writer.source_list();
    assert!(
        !serde_json::to_string(&source_list)
            .expect("source list json string")
            .contains(&fs_id),
        "AgentFS-backed source must not be exposed through source list"
    );
    assert!(
        !serde_json::to_string(&source_list)
            .expect("source list json string")
            .contains(fixture.remote_root.to_str().expect("utf8 remote root")),
        "source list JSON must not expose AgentFS backing root"
    );
    let public_status = writer.runtime_status();
    let public_status_json = serde_json::to_string(&public_status).expect("runtime status json");
    assert!(
        !public_status_json.contains(&fs_id),
        "section status JSON must not expose AgentFS-backed source"
    );
    assert!(
        !public_status_json.contains(fixture.remote_root.to_str().expect("utf8 remote root")),
        "section status JSON must not expose AgentFS backing root"
    );

    let marker = read_json(writer.local_root.join(".section/root.json"));
    assert_eq!(marker["schema_version"], 1);
    assert_eq!(marker["source_id"], fs_id);
    assert_eq!(marker["fs_id"], fs_id);
    assert_eq!(marker["source_profile_id"], source_profile_id);
    assert_eq!(marker["agent_id"], writer_id);
    assert_eq!(marker["installation_id"], writer_installation_id);
    assert!(marker["base_commit_id"].is_null());
    assert!(marker["control_plane_endpoint"]
        .as_str()
        .expect("endpoint")
        .contains("section-control-service:file:"));
    assert!(
        !writer.local_root.join(".section/agentfs").exists(),
        "AgentFS metadata mirror must not sync as user content"
    );

    writer.write_local("docs/note.txt", "hello from writer");
    fs::create_dir_all(writer.local_root.join(".section/agentfs")).expect("create local metadata");
    fs::write(
        writer.local_root.join(".section/agentfs/fs.json"),
        r#"{"malicious":true}"#,
    )
    .expect("write local metadata draft");
    assert!(
        !fixture.remote_path("docs/note.txt").exists(),
        "local draft must not be shared truth before commit"
    );

    let status = writer.commit_status();
    assert_eq!(status["status"]["stale"], false);
    assert!(status["status"]["dirty_paths"]
        .as_array()
        .expect("dirty paths")
        .iter()
        .any(|path| path["path"] == "docs/note.txt" && path["op"] == "create"));

    let commit = writer.commit_apply("add writer note");
    let commit_id = commit["commit"]["commit_id"]
        .as_str()
        .expect("commit id")
        .to_string();
    assert_eq!(commit["commit"]["agent_id"], writer_id);
    assert_eq!(commit["commit"]["authorized_by"]["grant_id"], grant_id);
    assert_eq!(commit["commit"]["materialization_state"], "materialized");
    assert!(commit["commit"]["paths"]
        .as_array()
        .expect("commit paths")
        .iter()
        .all(|path| !path["path"]
            .as_str()
            .expect("path")
            .starts_with(".section/")));
    assert_eq!(
        fs::read_to_string(fixture.remote_path("docs/note.txt")).expect("remote file"),
        "hello from writer"
    );

    let commit_record = read_json(
        fixture
            .remote_root
            .join(".section/agentfs/commits")
            .join(format!("{commit_id}.json")),
    );
    assert_eq!(commit_record["authorized_by"]["grant_id"], grant_id);
    let fs_json = read_json(fixture.remote_root.join(".section/agentfs/fs.json"));
    assert_eq!(fs_json["fs_id"], fs_id);
    assert!(fs_json["malicious"].is_null());
    let events = agentfs_event_kinds(&fixture.remote_root);
    assert!(events.contains(&"commit.accepted".to_string()));
    assert!(events.contains(&"commit.materialized".to_string()));

    let owner_fresh_root = fixture.root.join("owner-fresh-root");
    owner.attach_to(&owner_fresh_root);
    assert_eq!(
        fs::read_to_string(owner_fresh_root.join("docs/note.txt"))
            .expect("owner observes materialized file"),
        "hello from writer"
    );
    let owner_commit = owner.status(FS_NAME);
    assert_eq!(owner_commit["status"]["agent_id"], owner_id);
}

#[test]
fn client_seed_cannot_overwrite_existing_source_profile() {
    let fixture = Fixture::new();
    let alternate_remote = fixture.root.join("alternate-remote");
    fs::create_dir_all(&alternate_remote).expect("create alternate remote");
    let owner = fixture.agent("owner");
    let writer = fixture.agent_with_remote("writer", &alternate_remote);

    owner.login("owner");
    let create = owner.create_fs();
    let fs_id = create["fs"]["fs_id"].as_str().expect("fs id").to_string();

    let writer_id = login_agent_id(&writer.login("writer"));
    owner.grant(&writer_id, "writer");
    let share = owner.share(&writer_id);
    let share_id = share["share"]["share_id"].as_str().expect("share id");
    let available = writer.available();
    assert!(
        !serde_json::to_string(&available)
            .expect("available json")
            .contains(alternate_remote.to_str().expect("utf8 alternate remote")),
        "available JSON must not expose client-seeded backing root"
    );
    let accept = writer.accept(share_id);
    assert!(
        !serde_json::to_string(&accept)
            .expect("accept json")
            .contains(alternate_remote.to_str().expect("utf8 alternate remote")),
        "accept JSON must not expose client-seeded backing root"
    );

    writer.attach();
    writer.write_local("from-writer.txt", "must use service profile");
    writer.commit_apply("service profile remains authoritative");
    assert_eq!(
        fs::read_to_string(fixture.remote_path("from-writer.txt")).expect("read original remote"),
        "must use service profile"
    );
    assert!(
        !alternate_remote.join("from-writer.txt").exists(),
        "writer config must not rewrite the service-owned source profile"
    );
    assert_eq!(
        read_json(fixture.remote_root.join(".section/agentfs/fs.json"))["fs_id"],
        fs_id
    );
}

#[test]
fn reader_cannot_commit_and_ungranted_agent_cannot_attach() {
    let fixture = Fixture::new();
    let owner = fixture.agent("owner");
    let reader = fixture.agent("reader");
    let stranger = fixture.agent("stranger");

    owner.login("owner");
    let create = owner.create_fs();
    let fs_id = create["fs"]["fs_id"].as_str().expect("fs id").to_string();

    let reader_id = reader.login("reader")["agent"]["agent_id"]
        .as_str()
        .expect("reader id")
        .to_string();
    owner.grant(&reader_id, "reader");
    share_and_accept(&owner, &reader, &reader_id);
    reader.attach();
    let source_sync = reader.source_sync_output(&fs_id);
    assert!(
        !source_sync.status.success(),
        "reader unexpectedly synced AgentFS-backed source through low-level source command\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&source_sync.stdout),
        String::from_utf8_lossy(&source_sync.stderr)
    );
    let rebound_root = fixture.root.join("reader-rebound-root");
    let source_bind = reader.source_bind_output(&fs_id, &rebound_root);
    assert!(
        !source_bind.status.success(),
        "reader unexpectedly rebound AgentFS-backed source through low-level source command\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&source_bind.stdout),
        String::from_utf8_lossy(&source_bind.stderr)
    );
    let source_remove = reader.source_remove_output(&fs_id);
    assert!(
        !source_remove.status.success(),
        "reader unexpectedly removed AgentFS-backed source through low-level source command\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&source_remove.stdout),
        String::from_utf8_lossy(&source_remove.stderr)
    );
    assert!(
        reader.local_root.join(".section/root.json").exists(),
        "failed low-level source remove must not remove AgentFS root marker"
    );
    let write = reader.write_output(&format!("{fs_id}/bypass.txt"));
    assert!(
        !write.status.success(),
        "reader unexpectedly wrote through low-level file route\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&write.stdout),
        String::from_utf8_lossy(&write.stderr)
    );
    let path_inspect = reader.path_inspect_output();
    assert!(
        !path_inspect.status.success(),
        "reader unexpectedly used low-level path command on AgentFS root\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&path_inspect.stdout),
        String::from_utf8_lossy(&path_inspect.stderr)
    );
    assert!(
        !fixture.remote_path("bypass.txt").exists(),
        "low-level write must not materialize AgentFS content"
    );
    reader.write_local("reader-draft.txt", "reader local draft");
    assert_json_error(&reader.commit_apply_output("reader draft"), "grant_denied");
    assert!(
        !fixture.remote_path("reader-draft.txt").exists(),
        "reader draft must not materialize"
    );

    stranger.login("stranger");
    assert_json_error(
        &stranger.attach_output(&stranger.local_root),
        "grant_denied",
    );
    assert!(
        !stranger.local_root.join(".section/root.json").exists(),
        "failed attach must not write root marker"
    );
}

#[test]
fn config_source_named_like_agentfs_source_cannot_bypass_file_router() {
    let fixture = Fixture::new();
    let owner = fixture.agent("owner");
    let writer = fixture.agent("writer");

    owner.login("owner");
    let create = owner.create_fs();
    let fs_id = create["fs"]["fs_id"].as_str().expect("fs id").to_string();

    let writer_id = login_agent_id(&writer.login("writer"));
    owner.grant(&writer_id, "writer");
    share_and_accept(&owner, &writer, &writer_id);
    writer.attach();

    writer.append_config_source(&fs_id, &fixture.remote_root);
    let write = writer.write_output(&format!("{fs_id}/bypass.txt"));
    assert!(
        !write.status.success(),
        "writer unexpectedly wrote AgentFS content through config source collision\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&write.stdout),
        String::from_utf8_lossy(&write.stderr)
    );
    assert!(
        !fixture.remote_path("bypass.txt").exists(),
        "config source collision must not materialize AgentFS content"
    );
    let public_status = writer.runtime_status();
    let public_status_json = serde_json::to_string(&public_status).expect("status json");
    assert!(
        !public_status_json.contains(&fs_id),
        "public runtime status must filter config-defined AgentFS source names"
    );
}

#[test]
fn stale_writer_cannot_overwrite_new_truth() {
    let fixture = Fixture::new();
    let owner = fixture.agent("owner");
    let writer_a = fixture.agent("writer-a");
    let writer_b = fixture.agent("writer-b");

    owner.login("owner");
    owner.create_fs();

    let writer_a_id = writer_a.login("writer-a")["agent"]["agent_id"]
        .as_str()
        .expect("writer a id")
        .to_string();
    owner.grant(&writer_a_id, "writer");
    share_and_accept(&owner, &writer_a, &writer_a_id);
    writer_a.attach();

    let writer_b_id = writer_b.login("writer-b")["agent"]["agent_id"]
        .as_str()
        .expect("writer b id")
        .to_string();
    owner.grant(&writer_b_id, "writer");
    share_and_accept(&owner, &writer_b, &writer_b_id);
    writer_b.attach();

    writer_a.write_local("docs/shared.txt", "from writer a");
    let commit_a = writer_a.commit_apply("writer a update");
    let commit_a_id = commit_a["commit"]["commit_id"]
        .as_str()
        .expect("writer a commit id")
        .to_string();

    let marker_path = writer_b.local_root.join(".section/root.json");
    let mut marker = read_json(&marker_path);
    marker["base_commit_id"] = Value::String(commit_a_id.clone());
    fs::write(
        &marker_path,
        serde_json::to_vec_pretty(&marker).expect("marker json"),
    )
    .expect("tamper writer b marker");
    writer_b.write_local("docs/from-b.txt", "from writer b");
    assert_json_error(
        &writer_b.commit_apply_output("writer b update"),
        "stale_base",
    );
    assert!(
        !fixture.remote_path("docs/from-b.txt").exists(),
        "stale writer draft must not materialize"
    );
    let head = read_json(
        fixture
            .remote_root
            .join(".section/agentfs/heads/current.json"),
    );
    assert_eq!(head["commit_id"], commit_a_id);
}

#[test]
fn hardening_rejects_non_empty_backing_and_attach_roots() {
    let non_empty = Fixture::new();
    fs::write(non_empty.remote_path("preexisting.txt"), "not imported").expect("write remote file");
    let owner = non_empty.agent("owner");
    owner.login("owner");
    let create = owner.create_fs_output();
    assert!(
        !create.status.success(),
        "fs create unexpectedly accepted non-empty backing source\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&create.stdout),
        String::from_utf8_lossy(&create.stderr)
    );
    assert!(
        !non_empty
            .remote_root
            .join(".section/agentfs/fs.json")
            .exists(),
        "rejected create must not initialize mirror metadata"
    );

    let overlap = Fixture::new();
    let owner = overlap.agent("owner");
    owner.login("owner");
    owner.create_fs();
    let attach = owner.attach_output(&overlap.remote_root);
    assert!(
        !attach.status.success(),
        "fs attach unexpectedly accepted backing root as working root\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&attach.stdout),
        String::from_utf8_lossy(&attach.stderr)
    );
    assert!(
        !overlap.remote_root.join(".section/root.json").exists(),
        "failed attach must not write marker into backing source"
    );

    let non_empty_local = Fixture::new();
    let owner = non_empty_local.agent("owner");
    owner.login("owner");
    owner.create_fs();
    fs::create_dir_all(&owner.local_root).expect("create owner root");
    fs::write(owner.local_root.join("draft.txt"), "must stay local").expect("write draft");
    let attach = owner.attach_output(&owner.local_root);
    assert!(
        !attach.status.success(),
        "attach unexpectedly accepted non-empty working root\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&attach.stdout),
        String::from_utf8_lossy(&attach.stderr)
    );
    assert!(
        !non_empty_local.remote_path("draft.txt").exists(),
        "attach must not publish local drafts"
    );
}

#[test]
fn attach_rejects_nested_parent_or_child_roots() {
    let fixture = Fixture::new();
    let owner = fixture.agent("owner");
    owner.login("owner");
    owner.create_fs();
    let parent_root = fixture.root.join("roots");
    let nested_root = parent_root.join("nested");
    owner.attach_to(&nested_root);

    let child_root = nested_root.join("child-root");
    let child_attach = owner.attach_output(&child_root);
    assert!(
        !child_attach.status.success(),
        "attach unexpectedly accepted child root\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&child_attach.stdout),
        String::from_utf8_lossy(&child_attach.stderr)
    );

    let attach_parent = owner.attach_output(&parent_root);
    assert!(
        !attach_parent.status.success(),
        "attach unexpectedly accepted parent root overlapping existing root\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&attach_parent.stdout),
        String::from_utf8_lossy(&attach_parent.stderr)
    );
}
