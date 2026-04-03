use anyhow::Result;
use section_core::SectionConfig;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

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

    let spec = mount_command(config_path, mount_point);
    let mut cmd = Command::new(spec.program);
    cmd.args(&spec.args);

    let child = cmd.spawn();
    match child {
        Ok(_) => {
            println!("Section filesystem mounted at {}", mount_point.display());
            let _ = config;
        }
        Err(e) => {
            anyhow::bail!("Failed to launch {}: {e}. {}", spec.display(), mount_hint(),);
        }
    }

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
        "Make sure section-fuse is installed and macFUSE is available on this machine."
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

#[cfg(test)]
mod tests {
    use super::{mount_command, unmount_commands};
    use std::path::Path;

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
}
