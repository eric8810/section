use anyhow::Result;
use section_core::SectionConfig;
use std::path::Path;
use std::process::Command;

pub fn mount(config: &SectionConfig, mount_point: &Path) -> Result<()> {
    std::fs::create_dir_all(mount_point)?;

    // Launch section-fuse as a subprocess
    let mut cmd = Command::new("section-fuse");
    cmd.arg("--mount-point").arg(mount_point);

    if let Some(config_path) = std::env::var_os("SECTION_CONFIG") {
        cmd.arg("--config").arg(config_path);
    }

    let child = cmd.spawn();
    match child {
        Ok(_) => {
            println!("Section filesystem mounted at {}", mount_point.display());
            let _ = config;
        }
        Err(e) => {
            anyhow::bail!(
                "Failed to launch section-fuse: {e}. Is section-fuse installed?"
            );
        }
    }

    Ok(())
}

pub fn unmount(mount_point: &Path) -> Result<()> {
    let status = Command::new("fusermount3")
        .arg("-u")
        .arg(mount_point)
        .status()
        .or_else(|_| {
            Command::new("fusermount")
                .arg("-u")
                .arg(mount_point)
                .status()
        })?;

    if status.success() {
        println!("Unmounted {}", mount_point.display());
    } else {
        anyhow::bail!("Failed to unmount {}", mount_point.display());
    }

    Ok(())
}
