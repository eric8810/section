use super::mount::is_mount_active;
use anyhow::Result;
use section_core::{Router, SectionConfig, SectionError};
use section_provider::ProviderStore;
use serde_json::json;
use std::path::{Path, PathBuf};

const REFRESH_XATTR_NAME: &str = "section.refresh";
#[cfg(target_os = "linux")]
const REFRESH_XATTR_NAME_LINUX: &str = "user.section.refresh";

fn build_router(config: &SectionConfig, store: &ProviderStore) -> Result<Router> {
    let mut config = config.clone();
    let db_sources = store.load_all()?;
    for (name, source) in db_sources {
        config.sources.entry(name).or_insert(source);
    }
    Ok(Router::from_config(&config)?)
}

pub fn ls(
    config: &SectionConfig,
    store: &ProviderStore,
    path: Option<&str>,
    json_mode: bool,
) -> Result<()> {
    let router = build_router(config, store)?;

    let path = path.unwrap_or("").trim_matches('/');

    // Root level: list all sources
    if path.is_empty() {
        if json_mode {
            let arr: Vec<serde_json::Value> = router
                .sources()
                .into_iter()
                .map(|s| json!({"name": format!("{s}/"), "type": "directory"}))
                .collect();
            println!("{}", serde_json::to_string(&arr)?);
        } else {
            for source in router.sources() {
                println!("  {source}/");
            }
        }
        return Ok(());
    }

    // Check if path is just a source name (no sub-path)
    let (op, sub_path) = router.resolve(path)?;
    let sub_path = if sub_path.is_empty() || sub_path.ends_with('/') {
        sub_path
    } else {
        format!("{sub_path}/")
    };

    let rt = tokio::runtime::Runtime::new()?;
    let entries = rt.block_on(op.list(&sub_path))?;

    if json_mode {
        let arr: Vec<serde_json::Value> = entries
            .iter()
            .map(|entry| {
                let name = entry.name();
                if entry.metadata().is_dir() {
                    json!({"name": format!("{name}/"), "type": "directory"})
                } else {
                    let size = entry.metadata().content_length();
                    json!({"name": name, "type": "file", "size": size})
                }
            })
            .collect();
        println!("{}", serde_json::to_string(&arr)?);
    } else {
        for entry in entries {
            let name = entry.name();
            if entry.metadata().is_dir() {
                println!("  {name}/");
            } else {
                let size = entry.metadata().content_length();
                println!("  {name}  ({size} bytes)");
            }
        }
    }

    Ok(())
}

pub fn cat(
    config: &SectionConfig,
    store: &ProviderStore,
    path: &str,
    _json_mode: bool,
) -> Result<()> {
    let router = build_router(config, store)?;
    let (op, sub_path) = router.resolve(path)?;

    let rt = tokio::runtime::Runtime::new()?;
    let data = rt
        .block_on(op.read(&sub_path))
        .map_err(|e| SectionError::from_opendal(e, path))?;
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    std::io::Write::write_all(&mut handle, &data.to_vec())?;

    // cat outputs raw content to stdout. The JSON flag is accepted for
    // signature consistency but has no additional effect since the primary
    // output IS the file content.

    Ok(())
}

pub fn cp(
    config: &SectionConfig,
    store: &ProviderStore,
    src: &str,
    dst: &str,
    json_mode: bool,
) -> Result<()> {
    let router = build_router(config, store)?;
    let (src_op, src_path) = router.resolve(src)?;
    let (dst_op, dst_path) = router.resolve(dst)?;

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let data = src_op
            .read(&src_path)
            .await
            .map_err(|e| SectionError::from_opendal(e, src))?;
        dst_op
            .write(&dst_path, data)
            .await
            .map_err(|e| SectionError::from_opendal(e, dst))?;
        Ok::<_, SectionError>(())
    })?;

    if json_mode {
        println!(
            "{}",
            json!({"ok": true, "message": format!("Copied {src} -> {dst}")})
        );
    } else {
        println!("Copied {src} -> {dst}");
    }
    Ok(())
}

pub fn rm(
    config: &SectionConfig,
    store: &ProviderStore,
    path: &str,
    recursive: bool,
    json_mode: bool,
) -> Result<()> {
    let router = build_router(config, store)?;
    let (op, sub_path) = router.resolve(path)?;

    let rt = tokio::runtime::Runtime::new()?;

    if recursive {
        rt.block_on(op.remove_all(&sub_path))?;
    } else {
        rt.block_on(op.delete(&sub_path))?;
    }

    if json_mode {
        println!(
            "{}",
            json!({"ok": true, "message": format!("Removed {path}")})
        );
    } else {
        println!("Removed {path}");
    }
    Ok(())
}

fn refresh_attr_names() -> &'static [&'static str] {
    #[cfg(target_os = "linux")]
    {
        &[REFRESH_XATTR_NAME_LINUX, REFRESH_XATTR_NAME]
    }
    #[cfg(not(target_os = "linux"))]
    {
        &[REFRESH_XATTR_NAME]
    }
}

fn refresh_mount_target(mount_point: &Path, path: &str) -> PathBuf {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        mount_point.to_path_buf()
    } else {
        mount_point.join(trimmed)
    }
}

fn trigger_refresh_xattr(path: &Path) -> Result<Vec<u8>> {
    let mut last_error = None;

    for attr_name in refresh_attr_names() {
        match xattr::get(path, attr_name) {
            Ok(Some(data)) => return Ok(data),
            Ok(None) => {
                last_error = Some(anyhow::anyhow!(
                    "refresh xattr {attr_name} returned no data for {}",
                    path.display()
                ));
            }
            Err(err) => {
                last_error = Some(anyhow::anyhow!(
                    "failed to read refresh xattr {attr_name} on {}: {err}",
                    path.display()
                ));
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| anyhow::anyhow!("refresh xattr not available for {}", path.display())))
}

pub fn refresh(
    config: &SectionConfig,
    _store: &ProviderStore,
    path: &str,
    json_mode: bool,
) -> Result<()> {
    let mount_active = is_mount_active(&config.mount_point);
    let message = if mount_active {
        let mount_target = refresh_mount_target(&config.mount_point, path);
        let data = trigger_refresh_xattr(&mount_target)?;
        let response = String::from_utf8_lossy(&data);
        if response.trim().is_empty() || response.trim() == "ok" {
            format!("Cache refreshed for {path}")
        } else {
            format!("Cache refreshed for {path}: {}", response.trim())
        }
    } else {
        format!(
            "No active mount at {}; CLI has no persistent cache to invalidate for {path}",
            config.mount_point.display()
        )
    };

    if json_mode {
        println!(
            "{}",
            json!({
                "ok": true,
                "path": path,
                "mount_active": mount_active,
                "message": message,
            })
        );
    } else {
        println!("{message}");
    }

    Ok(())
}

pub fn write_stdin(
    config: &SectionConfig,
    store: &ProviderStore,
    path: &str,
    json_mode: bool,
) -> Result<()> {
    let router = build_router(config, store)?;
    let (op, sub_path) = router.resolve(path)?;

    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf)?;

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(op.write(&sub_path, buf))?;

    if json_mode {
        println!(
            "{}",
            json!({"ok": true, "message": format!("Written to {path}")})
        );
    } else {
        println!("Written to {path}");
    }
    Ok(())
}

pub fn exec(
    config: &SectionConfig,
    store: &ProviderStore,
    path: &str,
    args: &[String],
    json_mode: bool,
) -> Result<()> {
    let router = build_router(config, store)?;
    let (op, sub_path) = router.resolve(path)?;

    let rt = tokio::runtime::Runtime::new()?;
    let data = rt
        .block_on(op.read(&sub_path))
        .map_err(|e| SectionError::from_opendal(e, path))?;

    // Write to a temporary file
    let tmp_dir = std::env::temp_dir();
    let file_name = std::path::Path::new(&sub_path)
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("section-exec"));
    let tmp_path = tmp_dir.join(format!("section-exec-{}", file_name.to_string_lossy()));

    std::fs::write(&tmp_path, &data.to_vec())?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))?;
    }

    // Execute the script with inherited stdout/stderr
    let status = std::process::Command::new(&tmp_path)
        .args(args)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();

    // Clean up the temp file
    let _ = std::fs::remove_file(&tmp_path);

    match status {
        Ok(exit_status) => {
            let code = exit_status.code().unwrap_or(1);
            if code != 0 {
                if json_mode {
                    println!(
                        "{}",
                        json!({"ok": false, "error": format!("Process exited with code {code}")})
                    );
                }
                std::process::exit(code);
            }
            if json_mode {
                println!("{}", json!({"ok": true}));
            }
            Ok(())
        }
        Err(e) => {
            if json_mode {
                println!(
                    "{}",
                    json!({"ok": false, "error": format!("Failed to execute script: {e}")})
                );
                std::process::exit(1);
            }
            Err(anyhow::anyhow!("Failed to execute script: {}", e))
        }
    }
}
