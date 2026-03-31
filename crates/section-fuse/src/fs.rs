use crate::inode::{InodeTable, ROOT_INO};
use fuser::{
    FileType, Filesystem, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyOpen, ReplyWrite, Request, TimeOrNow,
};
use section_core::permission::Permission;
use section_core::Router;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::runtime::Runtime;

const TTL: Duration = Duration::from_secs(1);

/// Convert an OpenDAL error kind to the appropriate errno.
fn opendal_to_errno(err: &opendal::Error) -> i32 {
    match err.kind() {
        opendal::ErrorKind::NotFound => libc::ENOENT,
        opendal::ErrorKind::PermissionDenied => libc::EACCES,
        opendal::ErrorKind::AlreadyExists => libc::EEXIST,
        opendal::ErrorKind::RateLimited => libc::EAGAIN,
        opendal::ErrorKind::IsSameFile => libc::EINVAL,
        _ => libc::EIO,
    }
}

/// Buffered content for an open file handle.
struct OpenFile {
    /// Full section path (e.g. "my-s3/docs/file.txt").
    path: String,
    /// Buffered file content.
    data: Vec<u8>,
    /// Whether the buffer has been modified since open/last flush.
    dirty: bool,
}

/// Section FUSE filesystem implementation.
pub struct SectionFs {
    router: Router,
    inodes: InodeTable,
    rt: Runtime,
    next_fh: u64,
    open_files: HashMap<u64, OpenFile>,
}

impl SectionFs {
    pub fn new(router: Router) -> Self {
        let rt = Runtime::new().expect("failed to create tokio runtime");
        let mut inodes = InodeTable::new();

        // Pre-populate source directories under root.
        for source in router.sources() {
            inodes.ensure(
                ROOT_INO,
                &source,
                &source,
                FileType::Directory,
                0o755,
                0,
                SystemTime::UNIX_EPOCH,
            );
        }

        Self {
            router,
            inodes,
            rt,
            next_fh: 1,
            open_files: HashMap::new(),
        }
    }

    /// Build the child section-path from a parent inode and a child name.
    fn child_path(&self, parent: u64, name: &str) -> Option<String> {
        let parent_entry = self.inodes.get(parent)?;
        if parent_entry.path.is_empty() {
            Some(name.to_string())
        } else {
            Some(format!("{}/{}", parent_entry.path, name))
        }
    }

    /// Convert OpenDAL Metadata into (FileType, size, mtime).
    fn meta_to_tuple(meta: &opendal::Metadata) -> (FileType, u64, SystemTime) {
        let kind = if meta.is_dir() {
            FileType::Directory
        } else {
            FileType::RegularFile
        };
        let size = meta.content_length();
        let mtime = meta
            .last_modified()
            .map(SystemTime::from)
            .unwrap_or(UNIX_EPOCH);
        (kind, size, mtime)
    }

    /// Build a `Permission` from the attributes of a given inode.
    fn inode_permission(&self, ino: u64) -> Option<Permission> {
        self.inodes.get(ino).map(|e| Permission::new(e.attr.uid, e.attr.gid, e.attr.perm))
    }

    /// Flush dirty content for a file handle to the backend. Returns Ok on success.
    fn flush_fh(&mut self, fh: u64) -> Result<(), i32> {
        let file = match self.open_files.get(&fh) {
            Some(f) if f.dirty => f,
            _ => return Ok(()),
        };

        let path = file.path.clone();
        let data = file.data.clone();

        let parsed = Router::parse_path(&path).ok_or(libc::EIO)?;
        let op = self
            .router
            .get_operator(&parsed.source)
            .map_err(|_| libc::ENOENT)?
            .clone();

        self.rt
            .block_on(op.write(&parsed.sub_path, data))
            .map_err(|e| opendal_to_errno(&e))?;

        if let Some(f) = self.open_files.get_mut(&fh) {
            f.dirty = false;
        }
        Ok(())
    }
}

impl Filesystem for SectionFs {
    // ── getattr ──────────────────────────────────────────────────────────

    fn getattr(&mut self, _req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        match self.inodes.get(ino) {
            Some(entry) => reply.attr(&TTL, &entry.attr),
            None => reply.error(libc::ENOENT),
        }
    }

    // ── setattr ──────────────────────────────────────────────────────────

    fn setattr(
        &mut self,
        _req: &Request,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        if let Some(entry) = self.inodes.get_mut(ino) {
            if let Some(new_size) = size {
                entry.attr.size = new_size;
                entry.attr.blocks = (new_size + 511) / 512;
            }
            if let Some(new_mode) = mode {
                entry.attr.perm = new_mode as u16;
            }
            if let Some(new_uid) = uid {
                entry.attr.uid = new_uid;
            }
            if let Some(new_gid) = gid {
                entry.attr.gid = new_gid;
            }
            reply.attr(&TTL, &entry.attr);
        } else {
            reply.error(libc::ENOENT);
        }
    }

    // ── lookup ───────────────────────────────────────────────────────────

    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let name_str = name.to_string_lossy();

        // Fast path: cached inode.
        if let Some(ino) = self.inodes.lookup(parent, &name_str) {
            if let Some(entry) = self.inodes.get(ino) {
                reply.entry(&TTL, &entry.attr, 0);
                return;
            }
        }

        // Root → source directory lookup.
        if parent == ROOT_INO {
            if self.router.sources().iter().any(|s| s == name_str.as_ref()) {
                let ino = self.inodes.ensure(
                    ROOT_INO,
                    &name_str,
                    &name_str,
                    FileType::Directory,
                    0o755,
                    0,
                    SystemTime::UNIX_EPOCH,
                );
                reply.entry(&TTL, &self.inodes.get(ino).unwrap().attr, 0);
                return;
            }
            reply.error(libc::ENOENT);
            return;
        }

        // Deeper lookup – stat via OpenDAL.
        let child_path = match self.child_path(parent, &name_str) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let parsed = match Router::parse_path(&child_path) {
            Some(p) => p,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let op = match self.router.get_operator(&parsed.source) {
            Ok(o) => o.clone(),
            Err(_) => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let sub = parsed.sub_path.clone();

        // Try stat as file, fall back to directory.
        let result = self.rt.block_on(async {
            match op.stat(&sub).await {
                Ok(meta) => Ok(meta),
                Err(_) => {
                    let dir_path = if sub.ends_with('/') {
                        sub.clone()
                    } else {
                        format!("{}/", sub)
                    };
                    op.stat(&dir_path).await
                }
            }
        });

        match result {
            Ok(meta) => {
                let (kind, size, mtime) = Self::meta_to_tuple(&meta);
                let perm = if kind == FileType::Directory {
                    0o755
                } else {
                    0o644
                };
                let ino =
                    self.inodes
                        .ensure(parent, &name_str, &child_path, kind, perm, size, mtime);
                reply.entry(&TTL, &self.inodes.get(ino).unwrap().attr, 0);
            }
            Err(e) => reply.error(opendal_to_errno(&e)),
        }
    }

    // ── readdir ──────────────────────────────────────────────────────────

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let (parent_ino, path) = match self.inodes.get(ino) {
            Some(e) => (e.parent, e.path.clone()),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let mut entries: Vec<(u64, FileType, String)> = vec![
            (ino, FileType::Directory, ".".to_string()),
            (parent_ino, FileType::Directory, "..".to_string()),
        ];

        if ino == ROOT_INO {
            // List source directories.
            for source in self.router.sources() {
                let child_ino = self.inodes.ensure(
                    ROOT_INO,
                    &source,
                    &source,
                    FileType::Directory,
                    0o755,
                    0,
                    SystemTime::UNIX_EPOCH,
                );
                entries.push((child_ino, FileType::Directory, source));
            }
        } else {
            // List via OpenDAL.
            let parsed = match Router::parse_path(&path) {
                Some(p) => p,
                None => {
                    reply.error(libc::EIO);
                    return;
                }
            };

            let op = match self.router.get_operator(&parsed.source) {
                Ok(o) => o.clone(),
                Err(_) => {
                    reply.error(libc::ENOENT);
                    return;
                }
            };

            let list_path = if parsed.sub_path.is_empty() {
                "/".to_string()
            } else if parsed.sub_path.ends_with('/') {
                parsed.sub_path.clone()
            } else {
                format!("{}/", parsed.sub_path)
            };

            match self.rt.block_on(op.list(&list_path)) {
                Ok(opendal_entries) => {
                    for de in opendal_entries {
                        let child_name = de.name().trim_end_matches('/').to_string();
                        if child_name.is_empty() {
                            continue;
                        }

                        let child_path = format!("{}/{}", path, child_name);
                        let meta = de.metadata();
                        let (kind, size, mtime) = Self::meta_to_tuple(meta);
                        let perm = if kind == FileType::Directory {
                            0o755
                        } else {
                            0o644
                        };

                        let child_ino = self.inodes.ensure(
                            ino,
                            &child_name,
                            &child_path,
                            kind,
                            perm,
                            size,
                            mtime,
                        );
                        entries.push((child_ino, kind, child_name));
                    }
                }
                Err(e) => {
                    tracing::warn!("readdir failed for {}: {}", list_path, e);
                    reply.error(opendal_to_errno(&e));
                    return;
                }
            }
        }

        for (i, (child_ino, kind, name)) in
            entries.iter().enumerate().skip(offset as usize)
        {
            if reply.add(*child_ino, (i + 1) as i64, *kind, name) {
                break;
            }
        }
        reply.ok();
    }

    // ── open ─────────────────────────────────────────────────────────────

    fn open(&mut self, req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        let entry = match self.inodes.get(ino) {
            Some(e) => e,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        if entry.attr.kind == FileType::Directory {
            reply.error(libc::EISDIR);
            return;
        }

        // Permission check based on open flags.
        let perm = Permission::new(entry.attr.uid, entry.attr.gid, entry.attr.perm);
        let access_mode = flags & libc::O_ACCMODE;
        if access_mode == libc::O_RDONLY {
            if !perm.can_read(req.uid(), req.gid()) {
                reply.error(libc::EACCES);
                return;
            }
        } else if access_mode == libc::O_WRONLY {
            if !perm.can_write(req.uid(), req.gid()) {
                reply.error(libc::EACCES);
                return;
            }
        } else if access_mode == libc::O_RDWR {
            if !perm.can_read(req.uid(), req.gid()) || !perm.can_write(req.uid(), req.gid()) {
                reply.error(libc::EACCES);
                return;
            }
        }

        let path = entry.path.clone();

        let parsed = match Router::parse_path(&path) {
            Some(p) => p,
            None => {
                reply.error(libc::EIO);
                return;
            }
        };

        let op = match self.router.get_operator(&parsed.source) {
            Ok(o) => o.clone(),
            Err(_) => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // Read content from backend (for write-only opens on missing files, start empty).
        let data = match self.rt.block_on(op.read(&parsed.sub_path)) {
            Ok(buf) => buf.to_vec(),
            Err(e) => {
                let writing = flags & libc::O_WRONLY != 0 || flags & libc::O_RDWR != 0;
                if writing && e.kind() == opendal::ErrorKind::NotFound {
                    Vec::new()
                } else {
                    reply.error(opendal_to_errno(&e));
                    return;
                }
            }
        };

        let fh = self.next_fh;
        self.next_fh += 1;
        self.open_files.insert(
            fh,
            OpenFile {
                path,
                data,
                dirty: false,
            },
        );

        reply.opened(fh, 0);
    }

    // ── read ─────────────────────────────────────────────────────────────

    fn read(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let file = match self.open_files.get(&fh) {
            Some(f) => f,
            None => {
                reply.error(libc::EBADF);
                return;
            }
        };

        let offset = offset as usize;
        if offset >= file.data.len() {
            reply.data(&[]);
        } else {
            let end = (offset + size as usize).min(file.data.len());
            reply.data(&file.data[offset..end]);
        }
    }

    // ── write ────────────────────────────────────────────────────────────

    fn write(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        let file = match self.open_files.get_mut(&fh) {
            Some(f) => f,
            None => {
                reply.error(libc::EBADF);
                return;
            }
        };

        let offset = offset as usize;
        let end = offset + data.len();

        if end > file.data.len() {
            file.data.resize(end, 0);
        }
        file.data[offset..end].copy_from_slice(data);
        file.dirty = true;

        reply.written(data.len() as u32);
    }

    // ── flush ────────────────────────────────────────────────────────────

    fn flush(&mut self, _req: &Request, _ino: u64, fh: u64, _lock_owner: u64, reply: ReplyEmpty) {
        match self.flush_fh(fh) {
            Ok(_) => reply.ok(),
            Err(errno) => reply.error(errno),
        }
    }

    // ── release ──────────────────────────────────────────────────────────

    fn release(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        // Flush dirty data before releasing.
        if let Err(errno) = self.flush_fh(fh) {
            self.open_files.remove(&fh);
            reply.error(errno);
            return;
        }
        self.open_files.remove(&fh);
        reply.ok();
    }

    // ── create ───────────────────────────────────────────────────────────

    fn create(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        // Check write permission on parent directory.
        if let Some(perm) = self.inode_permission(parent) {
            if !perm.can_write(req.uid(), req.gid()) {
                reply.error(libc::EACCES);
                return;
            }
        }

        let name_str = name.to_string_lossy().to_string();
        let child_path = match self.child_path(parent, &name_str) {
            Some(p) => p,
            None => {
                reply.error(libc::EIO);
                return;
            }
        };

        let parsed = match Router::parse_path(&child_path) {
            Some(p) => p,
            None => {
                reply.error(libc::EPERM);
                return;
            }
        };

        let op = match self.router.get_operator(&parsed.source) {
            Ok(o) => o.clone(),
            Err(_) => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // Create an empty file on the backend.
        if let Err(e) = self.rt.block_on(op.write(&parsed.sub_path, Vec::<u8>::new())) {
            reply.error(opendal_to_errno(&e));
            return;
        }

        let now = SystemTime::now();
        let ino = self.inodes.ensure(
            parent,
            &name_str,
            &child_path,
            FileType::RegularFile,
            0o644,
            0,
            now,
        );

        let attr = self.inodes.get(ino).unwrap().attr;

        let fh = self.next_fh;
        self.next_fh += 1;
        self.open_files.insert(
            fh,
            OpenFile {
                path: child_path,
                data: Vec::new(),
                dirty: false,
            },
        );

        reply.created(&TTL, &attr, 0, fh, 0);
    }

    // ── mkdir ────────────────────────────────────────────────────────────

    fn mkdir(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        // Check write permission on parent directory.
        if let Some(perm) = self.inode_permission(parent) {
            if !perm.can_write(req.uid(), req.gid()) {
                reply.error(libc::EACCES);
                return;
            }
        }

        let name_str = name.to_string_lossy().to_string();
        let child_path = match self.child_path(parent, &name_str) {
            Some(p) => p,
            None => {
                reply.error(libc::EIO);
                return;
            }
        };

        let parsed = match Router::parse_path(&child_path) {
            Some(p) => p,
            None => {
                reply.error(libc::EPERM);
                return;
            }
        };

        let op = match self.router.get_operator(&parsed.source) {
            Ok(o) => o.clone(),
            Err(_) => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let dir_path = if parsed.sub_path.ends_with('/') {
            parsed.sub_path.clone()
        } else {
            format!("{}/", parsed.sub_path)
        };

        if let Err(e) = self.rt.block_on(op.create_dir(&dir_path)) {
            reply.error(opendal_to_errno(&e));
            return;
        }

        let now = SystemTime::now();
        let ino = self.inodes.ensure(
            parent,
            &name_str,
            &child_path,
            FileType::Directory,
            0o755,
            0,
            now,
        );

        reply.entry(&TTL, &self.inodes.get(ino).unwrap().attr, 0);
    }

    // ── unlink ───────────────────────────────────────────────────────────

    fn unlink(&mut self, req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        // Check write permission on parent directory.
        if let Some(perm) = self.inode_permission(parent) {
            if !perm.can_write(req.uid(), req.gid()) {
                reply.error(libc::EACCES);
                return;
            }
        }

        let name_str = name.to_string_lossy();

        let ino = match self.inodes.lookup(parent, &name_str) {
            Some(i) => i,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let path = match self.inodes.get(ino) {
            Some(e) => e.path.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let parsed = match Router::parse_path(&path) {
            Some(p) => p,
            None => {
                reply.error(libc::EPERM);
                return;
            }
        };

        let op = match self.router.get_operator(&parsed.source) {
            Ok(o) => o.clone(),
            Err(_) => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        match self.rt.block_on(op.delete(&parsed.sub_path)) {
            Ok(_) => {
                self.inodes.remove(ino);
                reply.ok();
            }
            Err(e) => reply.error(opendal_to_errno(&e)),
        }
    }

    // ── rmdir ────────────────────────────────────────────────────────────

    fn rmdir(&mut self, req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        // Check write permission on parent directory.
        if let Some(perm) = self.inode_permission(parent) {
            if !perm.can_write(req.uid(), req.gid()) {
                reply.error(libc::EACCES);
                return;
            }
        }

        let name_str = name.to_string_lossy();

        let ino = match self.inodes.lookup(parent, &name_str) {
            Some(i) => i,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let path = match self.inodes.get(ino) {
            Some(e) => e.path.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let parsed = match Router::parse_path(&path) {
            Some(p) => p,
            None => {
                reply.error(libc::EPERM);
                return;
            }
        };

        let op = match self.router.get_operator(&parsed.source) {
            Ok(o) => o.clone(),
            Err(_) => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let dir_path = if parsed.sub_path.ends_with('/') {
            parsed.sub_path.clone()
        } else {
            format!("{}/", parsed.sub_path)
        };

        match self.rt.block_on(op.delete(&dir_path)) {
            Ok(_) => {
                self.inodes.invalidate_children(ino);
                self.inodes.remove(ino);
                reply.ok();
            }
            Err(e) => reply.error(opendal_to_errno(&e)),
        }
    }

    // ── rename ───────────────────────────────────────────────────────────

    fn rename(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        _flags: u32,
        reply: ReplyEmpty,
    ) {
        // Check write permission on both old and new parent directories.
        if let Some(perm) = self.inode_permission(parent) {
            if !perm.can_write(req.uid(), req.gid()) {
                reply.error(libc::EACCES);
                return;
            }
        }
        if let Some(perm) = self.inode_permission(newparent) {
            if !perm.can_write(req.uid(), req.gid()) {
                reply.error(libc::EACCES);
                return;
            }
        }

        let old_name = name.to_string_lossy();
        let new_name = newname.to_string_lossy();

        let old_ino = match self.inodes.lookup(parent, &old_name) {
            Some(i) => i,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let old_path = match self.inodes.get(old_ino) {
            Some(e) => e.path.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let new_path = match self.child_path(newparent, &new_name) {
            Some(p) => p,
            None => {
                reply.error(libc::EIO);
                return;
            }
        };

        let old_parsed = match Router::parse_path(&old_path) {
            Some(p) => p,
            None => {
                reply.error(libc::EPERM);
                return;
            }
        };
        let new_parsed = match Router::parse_path(&new_path) {
            Some(p) => p,
            None => {
                reply.error(libc::EPERM);
                return;
            }
        };

        // Cross-source rename is not supported.
        if old_parsed.source != new_parsed.source {
            reply.error(libc::EXDEV);
            return;
        }

        let op = match self.router.get_operator(&old_parsed.source) {
            Ok(o) => o.clone(),
            Err(_) => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // Emulate rename as copy + delete (most backends lack atomic rename).
        let result = self.rt.block_on(async {
            op.copy(&old_parsed.sub_path, &new_parsed.sub_path).await?;
            op.delete(&old_parsed.sub_path).await?;
            Ok::<_, opendal::Error>(())
        });

        match result {
            Ok(_) => {
                self.inodes.remove(old_ino);
                reply.ok();
            }
            Err(e) => reply.error(opendal_to_errno(&e)),
        }
    }
}
