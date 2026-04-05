use crate::SectionWorld;
use cucumber::when;

/// 当 我执行管道 "{input}" 到 "{command}"
#[when(expr = "我执行管道 {string} 到 {string}")]
fn execute_pipe_command(world: &mut SectionWorld, input_cmd: String, section_cmd: String) {
    let section_cmd = section_cmd.strip_prefix("section ").unwrap_or(&section_cmd);
    let args = crate::shell_words(section_cmd);

    let bin = env!("CARGO_BIN_EXE_section");
    let config_path = world.data_dir_path().join("config.toml");

    // Run input command to get its output
    let input_output = std::process::Command::new("bash")
        .arg("-c")
        .arg(&input_cmd)
        .output()
        .expect("failed to run input command");

    // Pipe it to section command
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(bin);
    cmd.arg("--config")
        .arg(&config_path)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn section command");

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&input_output.stdout)
            .expect("failed to write to stdin");
    }

    let output = child
        .wait_with_output()
        .expect("failed to wait for section command");

    world.last_stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.last_stderr = String::from_utf8_lossy(&output.stderr).to_string();
    world.last_success = output.status.success();
}
