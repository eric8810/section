use crate::support::actor::Actor;
use crate::support::write_config;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Fixture {
    _temp: tempfile::TempDir,
    remote_root: PathBuf,
    actors: Vec<Actor>,
}

impl Fixture {
    pub fn fs_with_actors(count: usize) -> Self {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let remote_root = temp.path().join("remote-root");
        fs::create_dir_all(&remote_root).expect("create remote root");

        let actors = (0..count)
            .map(|index| {
                let actor_root = temp.path().join(format!("actor-{index}"));
                let data_dir = actor_root.join("data");
                let config_path = actor_root.join("config.toml");
                let local_root = actor_root.join("local-root");
                fs::create_dir_all(&actor_root).expect("create actor root");
                write_config(&config_path, &data_dir);
                Actor::new(config_path, local_root, "shared")
            })
            .collect();

        Self {
            _temp: temp,
            remote_root,
            actors,
        }
    }

    pub fn remote_root(&self) -> &Path {
        &self.remote_root
    }

    pub fn actor(&self, index: usize) -> &Actor {
        &self.actors[index]
    }

    pub fn seed_remote(&self, relative_path: &str, content: &str) {
        let remote_path = self.remote_path(relative_path);
        if let Some(parent) = remote_path.parent() {
            fs::create_dir_all(parent).expect("create remote parent");
        }
        fs::write(remote_path, content).expect("write remote file");
    }

    pub fn read_remote(&self, relative_path: &str) -> String {
        fs::read_to_string(self.remote_path(relative_path)).expect("read remote file")
    }

    pub fn remote_path(&self, relative_path: &str) -> PathBuf {
        self.remote_root.join(relative_path)
    }
}
