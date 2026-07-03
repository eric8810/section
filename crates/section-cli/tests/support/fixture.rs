use crate::support::actor::Actor;
use crate::support::environment::{EnvironmentProfile, ProviderProfile, S3Profile, SourceSpec};
use crate::support::write_config;
use opendal::services;
use opendal::Operator;
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Fixture {
    _temp: tempfile::TempDir,
    rt: tokio::runtime::Runtime,
    remote: Operator,
    source: SourceSpec,
    actors: Vec<Actor>,
}

impl Fixture {
    pub fn for_environment(profile: &EnvironmentProfile, count: usize) -> Self {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let rt = tokio::runtime::Runtime::new().expect("test runtime");
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

        let (remote, source) = match &profile.provider {
            ProviderProfile::Fs => build_fs_remote(&temp),
            ProviderProfile::S3Compatible(config) => build_s3_remote(config),
        };

        Self {
            _temp: temp,
            rt,
            remote,
            source,
            actors,
        }
    }

    pub fn fs_with_actors(count: usize) -> Self {
        Self::for_environment(&EnvironmentProfile::fs(), count)
    }

    pub fn source(&self) -> &SourceSpec {
        &self.source
    }

    pub fn actor(&self, index: usize) -> &Actor {
        &self.actors[index]
    }

    pub fn seed_remote(&self, relative_path: &str, content: &str) {
        let remote = self.remote.clone();
        let path = relative_path.to_string();
        let content = content.to_string();
        self.rt
            .block_on(async move { remote.write(&path, content).await })
            .expect("write remote file");
    }

    pub fn read_remote(&self, relative_path: &str) -> String {
        let remote = self.remote.clone();
        let path = relative_path.to_string();
        let buffer = self
            .rt
            .block_on(async move { remote.read(&path).await })
            .expect("read remote file");
        String::from_utf8(buffer.to_bytes().to_vec()).expect("utf8 remote file")
    }
}

fn build_fs_remote(temp: &tempfile::TempDir) -> (Operator, SourceSpec) {
    let remote_root = temp.path().join("remote-root");
    fs::create_dir_all(&remote_root).expect("create remote root");

    let builder = services::Fs::default().root(remote_root.to_str().expect("utf8 remote root"));
    let remote = Operator::new(builder).expect("fs operator").finish();

    let source = SourceSpec::new("fs").with_option("root", remote_root.to_string_lossy());
    (remote, source)
}

fn build_s3_remote(config: &S3Profile) -> (Operator, SourceSpec) {
    best_effort_ensure_s3_bucket(config);
    let root_prefix = unique_root_prefix();

    let mut builder = services::S3::default()
        .bucket(&config.bucket)
        .root(&root_prefix)
        .endpoint(&config.endpoint)
        .region(&config.region)
        .access_key_id(&config.access_key_id)
        .secret_access_key(&config.secret_access_key)
        .disable_config_load()
        .disable_ec2_metadata();
    if config.enable_virtual_host_style {
        builder = builder.enable_virtual_host_style();
    }

    let remote = Operator::new(builder).expect("s3 operator").finish();

    let mut source = SourceSpec::new("s3")
        .with_option("bucket", &config.bucket)
        .with_option("region", &config.region)
        .with_option("endpoint", &config.endpoint)
        .with_option("access_key_id", &config.access_key_id)
        .with_option("secret_access_key", &config.secret_access_key)
        .with_option("root", &root_prefix);
    if config.enable_virtual_host_style {
        source = source.with_option("enable_virtual_host_style", "true");
    }

    (remote, source)
}

fn unique_root_prefix() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time");
    format!("section-tests/{}/{}", std::process::id(), now.as_nanos())
}

fn best_effort_ensure_s3_bucket(config: &S3Profile) {
    let script = r#"
import os
import boto3
from botocore.config import Config
from botocore.exceptions import ClientError

cfg = None
if os.environ.get("SECTION_TEST_S3_ENABLE_VIRTUAL_HOST_STYLE") != "1":
    cfg = Config(s3={"addressing_style": "path"})

client = boto3.client(
    "s3",
    endpoint_url=os.environ["SECTION_TEST_S3_ENDPOINT"],
    aws_access_key_id=os.environ["SECTION_TEST_S3_ACCESS_KEY_ID"],
    aws_secret_access_key=os.environ["SECTION_TEST_S3_SECRET_ACCESS_KEY"],
    region_name=os.environ["SECTION_TEST_S3_REGION"],
    config=cfg,
)

bucket = os.environ["SECTION_TEST_S3_BUCKET"]
try:
    client.head_bucket(Bucket=bucket)
except ClientError:
    kwargs = {"Bucket": bucket}
    region = os.environ["SECTION_TEST_S3_REGION"]
    if region != "us-east-1":
        kwargs["CreateBucketConfiguration"] = {"LocationConstraint": region}
    client.create_bucket(**kwargs)
"#;

    let _ = Command::new("python3")
        .arg("-c")
        .arg(script)
        .env("SECTION_TEST_S3_ENDPOINT", &config.endpoint)
        .env("SECTION_TEST_S3_BUCKET", &config.bucket)
        .env("SECTION_TEST_S3_REGION", &config.region)
        .env("SECTION_TEST_S3_ACCESS_KEY_ID", &config.access_key_id)
        .env(
            "SECTION_TEST_S3_SECRET_ACCESS_KEY",
            &config.secret_access_key,
        )
        .env(
            "SECTION_TEST_S3_ENABLE_VIRTUAL_HOST_STYLE",
            if config.enable_virtual_host_style {
                "1"
            } else {
                "0"
            },
        )
        .output();
}
