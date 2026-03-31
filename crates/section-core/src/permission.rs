use serde::{Deserialize, Serialize};

/// POSIX-style permission for a path in the section filesystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permission {
    pub uid: u32,
    pub gid: u32,
    pub mode: u16, // e.g., 0o750
}

impl Permission {
    pub fn new(uid: u32, gid: u32, mode: u16) -> Self {
        Self { uid, gid, mode }
    }

    /// Default permission for account root directories.
    pub fn account_root(uid: u32, gid: u32) -> Self {
        Self { uid, gid, mode: 0o750 }
    }

    /// Default permission for files.
    pub fn default_file(uid: u32, gid: u32) -> Self {
        Self { uid, gid, mode: 0o640 }
    }

    /// Default permission for directories.
    pub fn default_dir(uid: u32, gid: u32) -> Self {
        Self { uid, gid, mode: 0o750 }
    }

    /// Check if the given uid/gid has read access.
    pub fn can_read(&self, uid: u32, gid: u32) -> bool {
        if uid == 0 {
            return true;
        }
        if uid == self.uid {
            return self.mode & 0o400 != 0;
        }
        if gid == self.gid {
            return self.mode & 0o040 != 0;
        }
        self.mode & 0o004 != 0
    }

    /// Check if the given uid/gid has write access.
    pub fn can_write(&self, uid: u32, gid: u32) -> bool {
        if uid == 0 {
            return true;
        }
        if uid == self.uid {
            return self.mode & 0o200 != 0;
        }
        if gid == self.gid {
            return self.mode & 0o020 != 0;
        }
        self.mode & 0o002 != 0
    }

    /// Check if the given uid/gid has execute access.
    pub fn can_execute(&self, uid: u32, gid: u32) -> bool {
        if uid == 0 {
            return true;
        }
        if uid == self.uid {
            return self.mode & 0o100 != 0;
        }
        if gid == self.gid {
            return self.mode & 0o010 != 0;
        }
        self.mode & 0o001 != 0
    }
}
