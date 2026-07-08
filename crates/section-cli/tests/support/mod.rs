#![allow(dead_code)]

pub mod actor;
pub mod check;
pub mod environment;
pub mod fixture;
pub mod scenario;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

pub fn write_config(path: &Path, data_dir: &Path) {
    fs::write(
        path,
        format!(
            "data_dir = {:?}\nmount_point = \"/tmp/section-mount-test\"\n",
            data_dir.to_string_lossy()
        ),
    )
    .expect("write config");
}

pub fn write_agentfs_config(
    path: &Path,
    data_dir: &Path,
    control_service_path: &Path,
    source_profile_name: &str,
    remote_root: &Path,
) {
    fs::write(
        path,
        format!(
            "data_dir = {:?}\nmount_point = \"/tmp/section-mount-test\"\n\n[control_service]\npath = {:?}\n\n[control_service.source_profiles.{:?}]\nprovider = \"fs\"\n\n[control_service.source_profiles.{:?}.options]\nroot = {:?}\n",
            data_dir.to_string_lossy(),
            control_service_path.to_string_lossy(),
            source_profile_name,
            source_profile_name,
            remote_root.to_string_lossy(),
        ),
    )
    .expect("write AgentFS config");
}

pub fn write_agentfs_endpoint_config(path: &Path, data_dir: &Path, endpoint: &str) {
    fs::write(
        path,
        format!(
            "data_dir = {:?}\nmount_point = \"/tmp/section-mount-test\"\n\n[control_service]\nendpoint = {:?}\n",
            data_dir.to_string_lossy(),
            endpoint,
        ),
    )
    .expect("write AgentFS endpoint config");
}

pub fn run_section(config_path: &Path, args: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_section");
    Command::new(bin)
        .arg("--config")
        .arg(config_path)
        .args(args)
        .output()
        .expect("run section")
}

pub fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
