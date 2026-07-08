use anyhow::{anyhow, bail, Result};
use futures::TryStreamExt;
use opendal::{Entry, Metadata, Operator};
use section_core::{Router, SectionConfig, SectionError};
use section_provider::ProviderStore;
use sectiond::SectiondControlPlane;
use serde_json::json;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn build_router(config: &SectionConfig, store: &ProviderStore) -> Result<Router> {
    let mut public_config = config.clone();
    let config_source_names = public_config.sources.keys().cloned().collect::<Vec<_>>();
    for name in config_source_names {
        if store.is_agentfs_source(&name)? {
            public_config.sources.remove(&name);
        }
    }
    for (name, source) in store.load_all()? {
        if !store.is_agentfs_source(&name)? {
            public_config.sources.entry(name).or_insert(source);
        }
    }
    Ok(Router::from_config(&public_config)?)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CopyKind {
    File,
    Dir,
}

enum CopyLocation<'a> {
    Source {
        op: &'a Operator,
        raw: &'a str,
        sub_path: String,
    },
    Local {
        raw: &'a str,
        path: PathBuf,
    },
}

impl CopyLocation<'_> {
    fn display(&self) -> &str {
        match self {
            CopyLocation::Source { raw, .. } | CopyLocation::Local { raw, .. } => raw,
        }
    }

    fn file_name(&self) -> Result<String> {
        match self {
            CopyLocation::Source { raw, sub_path, .. } => path_last_segment(sub_path)
                .or_else(|| path_last_segment(raw))
                .map(str::to_string),
            CopyLocation::Local { path, .. } => path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string),
        }
        .ok_or_else(|| anyhow!("unable to determine a file name for {}", self.display()))
    }
}

fn is_source_path(router: &Router, path: &str) -> bool {
    if path.is_empty()
        || path.starts_with("./")
        || path.starts_with("../")
        || path == "."
        || path == ".."
        || path.starts_with("~/")
        || Path::new(path).is_absolute()
    {
        return false;
    }

    let trimmed = path.trim_start_matches('/');
    let Some(source) = trimmed.split('/').next() else {
        return false;
    };

    !source.is_empty() && router.get_operator(source).is_ok()
}

fn resolve_copy_location<'a>(router: &'a Router, raw: &'a str) -> Result<CopyLocation<'a>> {
    if is_source_path(router, raw) {
        let (op, sub_path) = router.resolve(raw)?;
        Ok(CopyLocation::Source { op, raw, sub_path })
    } else {
        Ok(CopyLocation::Local {
            raw,
            path: PathBuf::from(raw),
        })
    }
}

fn metadata_kind(meta: &Metadata, path: &str) -> Result<CopyKind> {
    if meta.is_dir() {
        Ok(CopyKind::Dir)
    } else if meta.is_file() {
        Ok(CopyKind::File)
    } else {
        bail!("unsupported entry type for {path}")
    }
}

fn raw_has_trailing_separator(raw: &str) -> bool {
    raw.ends_with('/') || raw.ends_with(std::path::MAIN_SEPARATOR)
}

fn path_last_segment(path: &str) -> Option<&str> {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
}

fn normalized_source_dir(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}/")
    }
}

fn join_source_path(base: &str, child: &str) -> String {
    let base = base.trim_matches('/');
    let child = child.trim_matches('/');

    match (base.is_empty(), child.is_empty()) {
        (true, true) => String::new(),
        (true, false) => child.to_string(),
        (false, true) => base.to_string(),
        (false, false) => format!("{base}/{child}"),
    }
}

fn source_parent_dir(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    trimmed
        .rsplit_once('/')
        .map(|(parent, _)| normalized_source_dir(parent))
        .filter(|parent| !parent.is_empty())
}

fn source_path_with_relative(root: &str, relative: &str, kind: CopyKind) -> String {
    let joined = join_source_path(root, relative);
    if kind == CopyKind::Dir {
        normalized_source_dir(&joined)
    } else {
        joined
    }
}

fn relative_to_source_root(root: &str, path: &str) -> String {
    let root = normalized_source_dir(root);
    if root.is_empty() {
        path.trim_matches('/').to_string()
    } else {
        path.strip_prefix(&root)
            .unwrap_or(path)
            .trim_matches('/')
            .to_string()
    }
}

fn source_stat(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    sub_path: &str,
    raw: &str,
) -> std::result::Result<Metadata, SectionError> {
    let mut last_not_found = None;

    for candidate in [sub_path.to_string(), normalized_source_dir(sub_path)] {
        if candidate.is_empty() {
            continue;
        }

        match rt.block_on(op.stat(&candidate)) {
            Ok(meta) => return Ok(meta),
            Err(err) if err.kind() == opendal::ErrorKind::NotFound => {
                last_not_found = Some(err);
            }
            Err(err) => return Err(SectionError::from_opendal(err, raw)),
        }
    }

    Err(SectionError::from_opendal(
        last_not_found
            .unwrap_or_else(|| opendal::Error::new(opendal::ErrorKind::NotFound, "path not found")),
        raw,
    ))
}

fn source_kind(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    sub_path: &str,
    raw: &str,
) -> Result<CopyKind> {
    if sub_path.is_empty() {
        return Ok(CopyKind::Dir);
    }

    metadata_kind(&source_stat(rt, op, sub_path, raw)?, raw)
}

fn local_kind(path: &Path, raw: &str) -> Result<CopyKind> {
    let meta =
        fs::metadata(path).map_err(|err| anyhow!("failed to read metadata for {raw}: {err}"))?;
    if meta.is_dir() {
        Ok(CopyKind::Dir)
    } else if meta.is_file() {
        Ok(CopyKind::File)
    } else {
        bail!("unsupported local file type for {raw}")
    }
}

fn copy_kind(rt: &tokio::runtime::Runtime, location: &CopyLocation<'_>) -> Result<CopyKind> {
    match location {
        CopyLocation::Source {
            op, raw, sub_path, ..
        } => source_kind(rt, op, sub_path, raw),
        CopyLocation::Local { raw, path } => local_kind(path, raw),
    }
}

fn ensure_local_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

fn ensure_source_parent(rt: &tokio::runtime::Runtime, op: &Operator, path: &str) -> Result<()> {
    if let Some(parent) = source_parent_dir(path) {
        rt.block_on(op.create_dir(&parent))?;
    }
    Ok(())
}

fn resolve_local_file_destination(raw: &str, path: &Path, file_name: &str) -> PathBuf {
    if path.is_dir() || raw_has_trailing_separator(raw) {
        path.join(file_name)
    } else {
        path.to_path_buf()
    }
}

fn resolve_source_file_destination(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    raw: &str,
    sub_path: &str,
    file_name: &str,
) -> Result<String> {
    if sub_path.is_empty() {
        return Ok(file_name.to_string());
    }

    match source_stat(rt, op, sub_path, raw) {
        Ok(meta) if meta.is_dir() => Ok(join_source_path(sub_path, file_name)),
        Ok(_) => Ok(sub_path.to_string()),
        Err(SectionError::FileNotFound(_)) | Err(SectionError::InvalidPath(_)) => {
            if raw_has_trailing_separator(raw) {
                Ok(join_source_path(sub_path, file_name))
            } else {
                Ok(sub_path.to_string())
            }
        }
        Err(err) => Err(err.into()),
    }
}

fn source_dir_destination_root(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    raw: &str,
    sub_path: &str,
) -> Result<String> {
    if sub_path.is_empty() {
        return Ok(String::new());
    }

    match source_stat(rt, op, sub_path, raw) {
        Ok(meta) if meta.is_file() => bail!(
            "destination {} is a file; directory copy requires a directory destination",
            raw
        ),
        Ok(_) | Err(SectionError::FileNotFound(_)) | Err(SectionError::InvalidPath(_)) => {
            Ok(sub_path.to_string())
        }
        Err(err) => Err(err.into()),
    }
}

fn local_dir_destination_root(raw: &str, path: &Path) -> Result<PathBuf> {
    if path.exists() && !path.is_dir() {
        bail!(
            "destination {} is a file; directory copy requires a directory destination",
            raw
        );
    }
    Ok(path.to_path_buf())
}

fn list_source_tree(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    root: &str,
    raw: &str,
) -> Result<Vec<(String, CopyKind)>> {
    let entries: Vec<Entry> = rt.block_on(async {
        op.list_with(&normalized_source_dir(root))
            .recursive(true)
            .await
    })?;
    let mut items = Vec::new();

    for entry in entries {
        let relative = relative_to_source_root(root, entry.path());
        if relative.is_empty() {
            continue;
        }
        items.push((relative, metadata_kind(entry.metadata(), raw)?));
    }

    items.sort_by(|(left, _), (right, _)| left.cmp(right));
    Ok(items)
}

fn list_local_tree(root: &Path) -> Result<Vec<(PathBuf, CopyKind)>> {
    fn walk(current: &Path, root: &Path, items: &mut Vec<(PathBuf, CopyKind)>) -> Result<()> {
        let mut entries = fs::read_dir(current)?.collect::<std::result::Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|err| {
                    anyhow!(
                        "failed to derive relative path for {}: {err}",
                        path.display()
                    )
                })?
                .to_path_buf();
            let metadata = entry.metadata()?;

            if metadata.is_dir() {
                items.push((relative.clone(), CopyKind::Dir));
                walk(&path, root, items)?;
            } else if metadata.is_file() {
                items.push((relative, CopyKind::File));
            } else {
                bail!("unsupported local file type for {}", path.display());
            }
        }

        Ok(())
    }

    let mut items = Vec::new();
    walk(root, root, &mut items)?;
    Ok(items)
}

fn relative_path_to_source(relative: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            std::path::Component::Normal(value) => parts.push(
                value
                    .to_str()
                    .ok_or_else(|| anyhow!("non-utf8 path component in {}", relative.display()))?
                    .to_string(),
            ),
            _ => bail!("unsupported path component in {}", relative.display()),
        }
    }
    Ok(parts.join("/"))
}

fn print_ls_entry(
    name: &str,
    kind: CopyKind,
    size: Option<u64>,
    modified: Option<&str>,
    long: bool,
) {
    if long {
        let entry_type = match kind {
            CopyKind::Dir => "dir",
            CopyKind::File => "file",
        };
        let size = size
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string());
        let modified = modified.unwrap_or("-");
        println!("{entry_type:<4} {size:>12} {modified:<30} {name}");
    } else if kind == CopyKind::Dir {
        println!("  {name}");
    } else {
        let size = size.unwrap_or_default();
        println!("  {name}  ({size} bytes)");
    }
}

pub fn ls(
    config: &SectionConfig,
    store: &ProviderStore,
    path: Option<&str>,
    json_mode: bool,
    long_mode: bool,
) -> Result<()> {
    let router = build_router(config, store)?;
    let path = path.unwrap_or("").trim_matches('/');

    if path.is_empty() {
        if json_mode {
            let arr: Vec<serde_json::Value> = router
                .sources()
                .into_iter()
                .map(|source| {
                    json!({
                        "name": format!("{source}/"),
                        "type": "directory",
                        "size": serde_json::Value::Null,
                        "last_modified": serde_json::Value::Null,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string(&arr)?);
        } else {
            for source in router.sources() {
                print_ls_entry(&format!("{source}/"), CopyKind::Dir, None, None, long_mode);
            }
        }
        return Ok(());
    }

    let (op, sub_path) = router.resolve(path)?;
    let sub_path = normalized_source_dir(&sub_path);
    let rt = tokio::runtime::Runtime::new()?;
    let mut entries = rt.block_on(op.list(&sub_path))?;
    entries.retain(|entry| !relative_to_source_root(&sub_path, entry.path()).is_empty());
    entries.sort_by_key(entry_name);
    let hydrate_metadata = json_mode || long_mode;

    if json_mode {
        let arr: Vec<serde_json::Value> = entries
            .iter()
            .map(|entry| {
                let meta = ls_entry_metadata(&rt, op, entry, hydrate_metadata);
                json!({
                    "name": entry_name(entry),
                    "type": if meta.is_dir() { "directory" } else { "file" },
                    "size": if meta.is_file() {
                        serde_json::Value::from(meta.content_length())
                    } else {
                        serde_json::Value::Null
                    },
                    "last_modified": meta
                        .last_modified()
                        .map(|ts| ts.to_string())
                        .map(serde_json::Value::from)
                        .unwrap_or(serde_json::Value::Null),
                })
            })
            .collect();
        println!("{}", serde_json::to_string(&arr)?);
    } else {
        for entry in entries {
            let meta = ls_entry_metadata(&rt, op, &entry, hydrate_metadata);
            let kind = metadata_kind(&meta, entry.path())?;
            let size = if kind == CopyKind::File {
                Some(meta.content_length())
            } else {
                None
            };
            let modified = meta.last_modified().map(|ts| ts.to_string());
            print_ls_entry(
                &entry_name(&entry),
                kind,
                size,
                modified.as_deref(),
                long_mode,
            );
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
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let reader = rt
        .block_on(op.reader(&sub_path))
        .map_err(|e| SectionError::from_opendal(e, path))?;
    let mut stream = rt
        .block_on(reader.into_bytes_stream(..))
        .map_err(|e| SectionError::from_opendal(e, path))?;

    while let Some(chunk) = rt
        .block_on(stream.try_next())
        .map_err(|e| anyhow!("failed to stream {path}: {e}"))?
    {
        handle.write_all(chunk.as_ref())?;
    }

    Ok(())
}

pub fn cp(
    config: &SectionConfig,
    store: &ProviderStore,
    src: &str,
    dst: &str,
    recursive: bool,
    json_mode: bool,
) -> Result<()> {
    let router = build_router(config, store)?;
    let src_location = resolve_copy_location(&router, src)?;
    let dst_location = resolve_copy_location(&router, dst)?;
    let rt = tokio::runtime::Runtime::new()?;

    match copy_kind(&rt, &src_location)? {
        CopyKind::File => {
            let src_name = src_location.file_name()?;
            match (&src_location, &dst_location) {
                (
                    CopyLocation::Source {
                        op: src_op,
                        sub_path: src_path,
                        ..
                    },
                    CopyLocation::Source {
                        op: dst_op,
                        raw: dst_raw,
                        sub_path: dst_path,
                    },
                ) => {
                    let target =
                        resolve_source_file_destination(&rt, dst_op, dst_raw, dst_path, &src_name)?;
                    let data = rt
                        .block_on(src_op.read(src_path))
                        .map_err(|e| SectionError::from_opendal(e, src))?;
                    ensure_source_parent(&rt, dst_op, &target)?;
                    rt.block_on(dst_op.write(&target, data))
                        .map_err(|e| SectionError::from_opendal(e, dst))?;
                }
                (
                    CopyLocation::Source {
                        op: src_op,
                        sub_path: src_path,
                        ..
                    },
                    CopyLocation::Local {
                        raw: dst_raw,
                        path: dst_path,
                    },
                ) => {
                    let target = resolve_local_file_destination(dst_raw, dst_path, &src_name);
                    ensure_local_parent(&target)?;
                    let data = rt
                        .block_on(src_op.read(src_path))
                        .map_err(|e| SectionError::from_opendal(e, src))?;
                    fs::write(&target, data.to_bytes())?;
                }
                (
                    CopyLocation::Local { path: src_path, .. },
                    CopyLocation::Source {
                        op: dst_op,
                        raw: dst_raw,
                        sub_path: dst_path,
                    },
                ) => {
                    let target =
                        resolve_source_file_destination(&rt, dst_op, dst_raw, dst_path, &src_name)?;
                    ensure_source_parent(&rt, dst_op, &target)?;
                    rt.block_on(dst_op.write(&target, fs::read(src_path)?))
                        .map_err(|e| SectionError::from_opendal(e, dst))?;
                }
                (
                    CopyLocation::Local { path: src_path, .. },
                    CopyLocation::Local {
                        raw: dst_raw,
                        path: dst_path,
                    },
                ) => {
                    let target = resolve_local_file_destination(dst_raw, dst_path, &src_name);
                    ensure_local_parent(&target)?;
                    fs::copy(src_path, &target)?;
                }
            }
        }
        CopyKind::Dir => {
            if !recursive {
                bail!("{} is a directory; use -r to copy recursively", src);
            }

            match (&src_location, &dst_location) {
                (
                    CopyLocation::Source {
                        op: src_op,
                        raw: src_raw,
                        sub_path: src_root,
                    },
                    CopyLocation::Source {
                        op: dst_op,
                        raw: dst_raw,
                        sub_path: dst_root,
                    },
                ) => {
                    let dst_root = source_dir_destination_root(&rt, dst_op, dst_raw, dst_root)?;
                    if !dst_root.is_empty() {
                        rt.block_on(dst_op.create_dir(&normalized_source_dir(&dst_root)))?;
                    }
                    for (relative, kind) in list_source_tree(&rt, src_op, src_root, src_raw)? {
                        let src_path = source_path_with_relative(src_root, &relative, kind);
                        let dst_path = source_path_with_relative(&dst_root, &relative, kind);
                        match kind {
                            CopyKind::Dir => {
                                rt.block_on(dst_op.create_dir(&dst_path))?;
                            }
                            CopyKind::File => {
                                ensure_source_parent(&rt, dst_op, &dst_path)?;
                                let data = rt
                                    .block_on(src_op.read(&src_path))
                                    .map_err(|e| SectionError::from_opendal(e, src))?;
                                rt.block_on(dst_op.write(&dst_path, data))
                                    .map_err(|e| SectionError::from_opendal(e, dst))?;
                            }
                        }
                    }
                }
                (
                    CopyLocation::Source {
                        op: src_op,
                        raw: src_raw,
                        sub_path: src_root,
                    },
                    CopyLocation::Local {
                        raw: dst_raw,
                        path: dst_root,
                    },
                ) => {
                    let dst_root = local_dir_destination_root(dst_raw, dst_root)?;
                    fs::create_dir_all(&dst_root)?;
                    for (relative, kind) in list_source_tree(&rt, src_op, src_root, src_raw)? {
                        let target = dst_root.join(&relative);
                        match kind {
                            CopyKind::Dir => fs::create_dir_all(&target)?,
                            CopyKind::File => {
                                ensure_local_parent(&target)?;
                                let src_path = source_path_with_relative(src_root, &relative, kind);
                                let data = rt
                                    .block_on(src_op.read(&src_path))
                                    .map_err(|e| SectionError::from_opendal(e, src))?;
                                fs::write(&target, data.to_bytes())?;
                            }
                        }
                    }
                }
                (
                    CopyLocation::Local {
                        raw: src_raw,
                        path: src_root,
                    },
                    CopyLocation::Source {
                        op: dst_op,
                        raw: dst_raw,
                        sub_path: dst_root,
                    },
                ) => {
                    let dst_root = source_dir_destination_root(&rt, dst_op, dst_raw, dst_root)?;
                    if !dst_root.is_empty() {
                        rt.block_on(dst_op.create_dir(&normalized_source_dir(&dst_root)))?;
                    }
                    for (relative, kind) in list_local_tree(src_root)? {
                        let relative = relative_path_to_source(&relative)?;
                        let dst_path = source_path_with_relative(&dst_root, &relative, kind);
                        match kind {
                            CopyKind::Dir => {
                                rt.block_on(dst_op.create_dir(&dst_path))?;
                            }
                            CopyKind::File => {
                                ensure_source_parent(&rt, dst_op, &dst_path)?;
                                let src_path = src_root
                                    .join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
                                rt.block_on(dst_op.write(&dst_path, fs::read(src_path)?))
                                    .map_err(|e| SectionError::from_opendal(e, dst))?;
                            }
                        }
                    }
                    let _ = src_raw;
                }
                (
                    CopyLocation::Local { path: src_root, .. },
                    CopyLocation::Local {
                        raw: dst_raw,
                        path: dst_root,
                    },
                ) => {
                    let dst_root = local_dir_destination_root(dst_raw, dst_root)?;
                    fs::create_dir_all(&dst_root)?;
                    for (relative, kind) in list_local_tree(src_root)? {
                        let target = dst_root.join(&relative);
                        match kind {
                            CopyKind::Dir => fs::create_dir_all(&target)?,
                            CopyKind::File => {
                                ensure_local_parent(&target)?;
                                fs::copy(src_root.join(&relative), &target)?;
                            }
                        }
                    }
                }
            }
        }
    }

    if json_mode {
        println!(
            "{}",
            json!({"ok": true, "recursive": recursive, "message": format!("Copied {src} -> {dst}")})
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

fn entry_name(entry: &Entry) -> String {
    let name = entry.name().trim_end_matches('/');
    if entry.metadata().is_dir() {
        format!("{name}/")
    } else {
        name.to_string()
    }
}

fn ls_entry_metadata(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    entry: &Entry,
    hydrate: bool,
) -> Metadata {
    if !hydrate {
        return entry.metadata().clone();
    }

    source_stat(rt, op, entry.path(), entry.path()).unwrap_or_else(|_| entry.metadata().clone())
}

pub fn refresh(config_path: Option<&Path>, path: &str, json_mode: bool) -> Result<()> {
    let result = SectiondControlPlane::load(config_path)?.refresh_path(path)?;

    if json_mode {
        println!(
            "{}",
            json!({
                "ok": true,
                "path": path,
                "mount_active": result.mount_active,
                "message": result.message,
            })
        );
    } else {
        println!("{}", result.message);
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
    std::io::stdin().read_to_end(&mut buf)?;

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

    std::fs::write(&tmp_path, data.to_vec())?;

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
