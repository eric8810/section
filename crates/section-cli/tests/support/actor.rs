use crate::support::{assert_success, run_section};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

#[derive(Debug, Clone)]
pub struct Actor {
    config_path: PathBuf,
    local_root: PathBuf,
    source_name: String,
}

impl Actor {
    pub fn new(config_path: PathBuf, local_root: PathBuf, source_name: impl Into<String>) -> Self {
        Self {
            config_path,
            local_root,
            source_name: source_name.into(),
        }
    }

    pub fn local_root(&self) -> &Path {
        &self.local_root
    }

    pub fn local_path(&self, relative_path: &str) -> PathBuf {
        self.local_root.join(relative_path)
    }

    pub fn add_fs_source(&self, remote_root: &Path) -> Output {
        let root_opt = format!("root={}", remote_root.display());
        let args = vec![
            "source".to_string(),
            "add".to_string(),
            self.source_name.clone(),
            "--provider".to_string(),
            "fs".to_string(),
            "--opt".to_string(),
            root_opt,
        ];
        self.run_owned(args)
    }

    pub fn bind(&self) -> Output {
        let local_root = self.local_root.to_str().expect("utf8 local root");
        let args = vec![
            "source".to_string(),
            "bind".to_string(),
            self.source_name.clone(),
            local_root.to_string(),
        ];
        self.run_owned(args)
    }

    pub fn connect_fs(&self, remote_root: &Path) {
        let add = self.add_fs_source(remote_root);
        assert_success(&add, "source add");
        let bind = self.bind();
        assert_success(&bind, "source bind");
    }

    pub fn sync(&self) -> Output {
        let args = vec![
            "source".to_string(),
            "sync".to_string(),
            self.source_name.clone(),
        ];
        self.run_owned(args)
    }

    pub fn sync_json(&self) -> Value {
        let args = vec![
            "--json".to_string(),
            "source".to_string(),
            "sync".to_string(),
            self.source_name.clone(),
        ];
        self.run_json_owned(args, "source sync")
    }

    pub fn compare(&self, relative_path: &str) -> Value {
        let local_path = self.local_path(relative_path);
        let local_path = local_path.to_str().expect("utf8 local path").to_string();
        let args = vec![
            "--json".to_string(),
            "path".to_string(),
            "compare".to_string(),
            local_path,
        ];
        self.run_json_owned(args, "path compare")
    }

    pub fn resolve(&self, relative_path: &str, strategy: &str) -> Value {
        let local_path = self.local_path(relative_path);
        let local_path = local_path.to_str().expect("utf8 local path").to_string();
        let args = vec![
            "--json".to_string(),
            "path".to_string(),
            "resolve".to_string(),
            local_path,
            "--strategy".to_string(),
            strategy.to_string(),
        ];
        self.run_json_owned(args, "path resolve")
    }

    pub fn watch_once(&self) -> Vec<Value> {
        let local_root = self
            .local_root
            .to_str()
            .expect("utf8 local root")
            .to_string();
        let args = vec![
            "--json".to_string(),
            "watch".to_string(),
            local_root,
            "--once".to_string(),
        ];
        let output = self.run_owned(args);
        assert_success(&output, "watch --once");
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("watch line json"))
            .collect()
    }

    pub fn write_local(&self, relative_path: &str, content: &str) {
        let local_path = self.local_path(relative_path);
        if let Some(parent) = local_path.parent() {
            fs::create_dir_all(parent).expect("create local parent");
        }
        fs::write(local_path, content).expect("write local file");
    }

    pub fn read_local(&self, relative_path: &str) -> String {
        fs::read_to_string(self.local_path(relative_path)).expect("read local file")
    }

    fn run_owned(&self, args: Vec<String>) -> Output {
        let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        run_section(&self.config_path, &refs)
    }

    fn run_json_owned(&self, args: Vec<String>, context: &str) -> Value {
        let output = self.run_owned(args);
        assert_success(&output, context);
        serde_json::from_slice(&output.stdout).expect("json output")
    }
}
