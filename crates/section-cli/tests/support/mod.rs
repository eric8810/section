#![allow(dead_code)]

pub mod actor;
pub mod check;
pub mod fixture;

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
