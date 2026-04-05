mod steps;

use cucumber::World;

#[derive(Debug, Default, World)]
pub struct SectionWorld {
    /// Temporary directory for Section data (config, db).
    data_dir: Option<tempfile::TempDir>,
    /// Last command's stdout output.
    last_stdout: String,
    /// Last command's stderr output.
    last_stderr: String,
    /// Last command's exit status (true = success).
    last_success: bool,
}

impl SectionWorld {
    fn data_dir_path(&self) -> &std::path::Path {
        self.data_dir
            .as_ref()
            .expect("data_dir not initialized")
            .path()
    }

    /// Build a `section` CLI command with the correct config pointing to our temp data dir.
    fn section_cmd(&self) -> std::process::Command {
        let bin = env!("CARGO_BIN_EXE_section");
        let mut cmd = std::process::Command::new(bin);
        // Point to a config that uses our temp data dir
        let config_path = self.data_dir_path().join("config.toml");
        cmd.arg("--config").arg(&config_path);
        cmd
    }

    /// Execute a section CLI command string and capture output.
    fn run_section_command(&mut self, args_str: &str) {
        // Parse the command string, stripping the leading "section " prefix
        let args_str = args_str.strip_prefix("section ").unwrap_or(args_str);
        let args = shell_words(args_str);

        let mut cmd = self.section_cmd();
        cmd.args(&args);

        let output = cmd.output().expect("failed to execute section command");

        self.last_stdout = String::from_utf8_lossy(&output.stdout).to_string();
        self.last_stderr = String::from_utf8_lossy(&output.stderr).to_string();
        self.last_success = output.status.success();
    }
}

/// Simple shell-like word splitting (handles --opt key=value and quoted strings).
fn shell_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    for ch in s.chars() {
        match ch {
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
            }
            ' ' | '\t' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn main() {
    let features_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("features");

    futures::executor::block_on(
        SectionWorld::cucumber()
            .with_default_cli()
            .run(features_path),
    );
}
