use anyhow::Result;
use section_core::SectionConfig;
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandSpec {
    program: &'static str,
    args: Vec<OsString>,
}

impl CommandSpec {
    fn new(program: &'static str, args: Vec<OsString>) -> Self {
        Self { program, args }
    }

    fn display(&self) -> String {
        let mut rendered = Vec::with_capacity(1 + self.args.len());
        rendered.push(self.program.to_string());
        rendered.extend(
            self.args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned()),
        );
        rendered.join(" ")
    }
}

pub fn mount(config: &SectionConfig, config_path: Option<&Path>, mount_point: &Path) -> Result<()> {
    std::fs::create_dir_all(mount_point)?;
    run_mount_preflight(mount_point)?;

    let spec = mount_command(config_path, mount_point);
    let mut cmd = Command::new(spec.program);
    cmd.args(&spec.args);

    let child = cmd.spawn().map_err(|e| {
        anyhow::anyhow!("Failed to launch {}: {e}. {}", spec.display(), mount_hint(),)
    })?;

    wait_for_mount_ready(child, mount_point)?;
    println!("Section filesystem mounted at {}", mount_point.display());
    let _ = config;

    Ok(())
}

pub fn unmount(mount_point: &Path) -> Result<()> {
    let mut failures = Vec::new();

    for spec in unmount_commands(mount_point) {
        match Command::new(spec.program).args(&spec.args).status() {
            Ok(status) if status.success() => {
                println!("Unmounted {}", mount_point.display());
                return Ok(());
            }
            Ok(status) => failures.push(format!("{} exited with {}", spec.display(), status)),
            Err(err) => failures.push(format!("{} failed to start: {}", spec.display(), err)),
        }
    }

    anyhow::bail!(
        "Failed to unmount {}. Tried: {}",
        mount_point.display(),
        failures.join("; ")
    )
}

fn mount_command(config_path: Option<&Path>, mount_point: &Path) -> CommandSpec {
    let mut args = vec![
        OsString::from("--mount-point"),
        mount_point.as_os_str().to_os_string(),
    ];

    if let Some(path) = config_path {
        args.push(OsString::from("--config"));
        args.push(path.as_os_str().to_os_string());
    } else if let Some(path) = std::env::var_os("SECTION_CONFIG") {
        args.push(OsString::from("--config"));
        args.push(path);
    }

    CommandSpec::new("section-fuse", args)
}

fn mount_hint() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Make sure section-fuse is installed, macFUSE is installed, and read docs/MACOS_ADAPTER.md for the supported setup path."
    }

    #[cfg(target_os = "linux")]
    {
        "Make sure section-fuse is installed and a FUSE runtime (for example fuse3) is available."
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        "Make sure section-fuse is installed and the current platform has a supported FUSE runtime."
    }
}

fn run_mount_preflight(mount_point: &Path) -> Result<()> {
    let mut issues = Vec::new();

    if !mount_point.is_absolute() {
        issues.push(format!("{} is not an absolute path", mount_point.display()));
    }

    if find_executable("section-fuse").is_none() {
        issues.push(
            "section-fuse is not available in PATH; install it (for example `cargo install --path crates/section-fuse`) or add the built binary directory to PATH"
                .to_string(),
        );
    }

    #[cfg(target_os = "macos")]
    {
        if !Path::new("/Library/Filesystems/macfuse.fs").exists() {
            issues.push(
                "macFUSE is not installed at /Library/Filesystems/macfuse.fs; install macFUSE, allow the system extension if prompted, and re-login/reboot before validating mount".to_string(),
            );
        }
    }

    #[cfg(target_os = "linux")]
    {
        if !Path::new("/dev/fuse").exists() {
            issues.push(
                "/dev/fuse is missing; install/enable a FUSE runtime such as fuse3 before validating mount"
                    .to_string(),
            );
        }
    }

    if issues.is_empty() {
        return Ok(());
    }

    anyhow::bail!(
        "Mount preflight failed for {}:\n- {}",
        mount_point.display(),
        issues.join("\n- ")
    )
}

pub(crate) fn is_mount_active(mount_point: &Path) -> bool {
    check_proc_mounts(mount_point) || check_mount_command(mount_point)
}

fn unmount_commands(mount_point: &Path) -> Vec<CommandSpec> {
    #[cfg(target_os = "linux")]
    {
        return vec![
            CommandSpec::new(
                "fusermount3",
                vec![OsString::from("-u"), mount_point.as_os_str().to_os_string()],
            ),
            CommandSpec::new(
                "fusermount",
                vec![OsString::from("-u"), mount_point.as_os_str().to_os_string()],
            ),
            CommandSpec::new("umount", vec![mount_point.as_os_str().to_os_string()]),
        ];
    }

    #[cfg(target_os = "macos")]
    {
        return vec![
            CommandSpec::new("umount", vec![mount_point.as_os_str().to_os_string()]),
            CommandSpec::new(
                "diskutil",
                vec![
                    OsString::from("unmount"),
                    OsString::from("force"),
                    mount_point.as_os_str().to_os_string(),
                ],
            ),
        ];
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        vec![CommandSpec::new(
            "umount",
            vec![mount_point.as_os_str().to_os_string()],
        )]
    }
}

fn wait_for_mount_ready(mut child: Child, mount_point: &Path) -> Result<()> {
    const READY_TIMEOUT: Duration = Duration::from_secs(5);
    const POLL_INTERVAL: Duration = Duration::from_millis(100);

    let deadline = Instant::now() + READY_TIMEOUT;

    loop {
        if is_mount_active(mount_point) {
            return Ok(());
        }

        if let Some(status) = child.try_wait()? {
            anyhow::bail!(
                "section-fuse exited before {} became an active mount (status: {}). {}",
                mount_point.display(),
                status,
                mount_hint(),
            );
        }

        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!(
                "Timed out waiting for {} to become an active mount. {}",
                mount_point.display(),
                mount_hint(),
            );
        }

        thread::sleep(POLL_INTERVAL);
    }
}

fn check_proc_mounts(mount_point: &Path) -> bool {
    let mount_str = mount_point.to_string_lossy();

    if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
        for line in mounts.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == mount_str.as_ref() {
                return true;
            }
        }
    }

    false
}

fn check_mount_command(mount_point: &Path) -> bool {
    let output = match Command::new("mount").output() {
        Ok(output) if output.status.success() => output,
        _ => return false,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    mount_output_contains_target(&stdout, mount_point)
}

fn mount_output_contains_target(output: &str, mount_point: &Path) -> bool {
    let mount_str = mount_point.to_string_lossy();

    output.lines().any(|line| {
        let Some((_, rest)) = line.split_once(" on ") else {
            return false;
        };

        rest == mount_str.as_ref()
            || rest.starts_with(&format!("{mount_str} "))
            || rest.starts_with(&format!("{mount_str} ("))
    })
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    find_executable_in_paths(name, env::split_paths(&path))
}

fn find_executable_in_paths<I>(name: &str, paths: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    for dir in paths {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{
        find_executable_in_paths, mount_command, mount_output_contains_target, unmount_commands,
    };
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn mount_command_forwards_explicit_config_path() {
        let spec = mount_command(
            Some(Path::new("/tmp/section.toml")),
            Path::new("/mnt/section"),
        );
        let args: Vec<String> = spec
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "--mount-point",
                "/mnt/section",
                "--config",
                "/tmp/section.toml",
            ]
        );
    }

    #[test]
    fn unmount_command_list_is_platform_aware() {
        let specs = unmount_commands(Path::new("/mnt/section"));
        assert!(!specs.is_empty());

        #[cfg(target_os = "linux")]
        assert_eq!(specs[0].program, "fusermount3");

        #[cfg(target_os = "macos")]
        assert_eq!(specs[0].program, "umount");
    }

    #[test]
    fn mount_output_parser_handles_linux_format() {
        let output = "section on /mnt/section type fuse.section (rw,nosuid,nodev,relatime)";
        assert!(mount_output_contains_target(
            output,
            Path::new("/mnt/section")
        ));
    }

    #[test]
    fn mount_output_parser_handles_macos_format() {
        let output = "osxfuse@macfuse0 on /Volumes/section (osxfuse, nodev, nosuid, synchronous)";
        assert!(mount_output_contains_target(
            output,
            Path::new("/Volumes/section")
        ));
    }

    #[test]
    fn executable_lookup_finds_binary_in_paths() {
        let temp_dir = TempDir::new().expect("temp dir");
        let bin = temp_dir.path().join("section-fuse");
        std::fs::write(&bin, "#!/bin/sh\n").expect("write file");

        let found = find_executable_in_paths(
            "section-fuse",
            vec![
                temp_dir.path().to_path_buf(),
                Path::new("/nope").to_path_buf(),
            ],
        );
        assert_eq!(found.as_deref(), Some(bin.as_path()));
    }
}
