use fuser::{FileAttr, FileType};
use std::collections::HashMap;
use std::time::SystemTime;

pub const ROOT_INO: u64 = 1;

pub struct InodeEntry {
    /// Full section path ("" for root, "source" for source dir, "source/sub/file" for deeper).
    pub path: String,
    /// Parent inode number.
    pub parent: u64,
    /// Entry name (last path component).
    pub name: String,
    /// Cached file attributes.
    pub attr: FileAttr,
}

pub struct InodeTable {
    next_ino: u64,
    pub entries: HashMap<u64, InodeEntry>,
    /// (parent_ino, child_name) -> child_ino
    lookup_map: HashMap<(u64, String), u64>,
}

impl InodeTable {
    pub fn new() -> Self {
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };

        let root_attr = FileAttr {
            ino: ROOT_INO,
            size: 0,
            blocks: 0,
            atime: SystemTime::UNIX_EPOCH,
            mtime: SystemTime::UNIX_EPOCH,
            ctime: SystemTime::UNIX_EPOCH,
            crtime: SystemTime::UNIX_EPOCH,
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid,
            gid,
            rdev: 0,
            blksize: 512,
            flags: 0,
        };

        let mut entries = HashMap::new();
        entries.insert(
            ROOT_INO,
            InodeEntry {
                path: String::new(),
                parent: ROOT_INO,
                name: String::new(),
                attr: root_attr,
            },
        );

        Self {
            next_ino: 2,
            entries,
            lookup_map: HashMap::new(),
        }
    }

    pub fn get(&self, ino: u64) -> Option<&InodeEntry> {
        self.entries.get(&ino)
    }

    pub fn get_mut(&mut self, ino: u64) -> Option<&mut InodeEntry> {
        self.entries.get_mut(&ino)
    }

    pub fn lookup(&self, parent: u64, name: &str) -> Option<u64> {
        self.lookup_map.get(&(parent, name.to_string())).copied()
    }

    /// Allocate or update an inode for the given parent + name.
    #[allow(clippy::too_many_arguments)]
    pub fn ensure(
        &mut self,
        parent: u64,
        name: &str,
        path: &str,
        kind: FileType,
        perm: u16,
        size: u64,
        mtime: SystemTime,
    ) -> u64 {
        if let Some(&ino) = self.lookup_map.get(&(parent, name.to_string())) {
            if let Some(entry) = self.entries.get_mut(&ino) {
                entry.attr.size = size;
                entry.attr.blocks = size.div_ceil(512);
                entry.attr.mtime = mtime;
                entry.attr.atime = mtime;
                entry.attr.kind = kind;
                entry.attr.perm = perm;
            }
            return ino;
        }

        let ino = self.next_ino;
        self.next_ino += 1;

        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };

        let attr = FileAttr {
            ino,
            size,
            blocks: size.div_ceil(512),
            atime: mtime,
            mtime,
            ctime: mtime,
            crtime: mtime,
            kind,
            perm,
            nlink: if kind == FileType::Directory { 2 } else { 1 },
            uid,
            gid,
            rdev: 0,
            blksize: 512,
            flags: 0,
        };

        self.entries.insert(
            ino,
            InodeEntry {
                path: path.to_string(),
                parent,
                name: name.to_string(),
                attr,
            },
        );
        self.lookup_map.insert((parent, name.to_string()), ino);

        ino
    }

    /// Remove an inode and its lookup entry.
    pub fn remove(&mut self, ino: u64) {
        if let Some(entry) = self.entries.remove(&ino) {
            self.lookup_map.remove(&(entry.parent, entry.name));
        }
    }

    /// Get all child inodes of a parent.
    pub fn children(&self, parent: u64) -> Vec<u64> {
        self.lookup_map
            .iter()
            .filter(|((p, _), _)| *p == parent)
            .map(|(_, &ino)| ino)
            .collect()
    }

    /// Remove all children of a parent inode.
    pub fn invalidate_children(&mut self, parent: u64) {
        let children: Vec<u64> = self.children(parent);
        for ino in children {
            self.remove(ino);
        }
    }
}
