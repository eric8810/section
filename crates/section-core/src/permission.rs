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
        Self {
            uid,
            gid,
            mode: 0o750,
        }
    }

    /// Default permission for files.
    pub fn default_file(uid: u32, gid: u32) -> Self {
        Self {
            uid,
            gid,
            mode: 0o640,
        }
    }

    /// Default permission for directories.
    pub fn default_dir(uid: u32, gid: u32) -> Self {
        Self {
            uid,
            gid,
            mode: 0o750,
        }
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

#[cfg(test)]
mod tests {
    use super::Permission;

    #[test]
    fn root_user_bypasses_permission_bits() {
        let permission = Permission::new(1000, 1000, 0o000);

        assert!(permission.can_read(0, 0));
        assert!(permission.can_write(0, 0));
        assert!(permission.can_execute(0, 0));
    }

    #[test]
    fn owner_group_and_other_bits_are_evaluated_independently() {
        let permission = Permission::new(1000, 2000, 0o754);

        assert!(permission.can_read(1000, 9999));
        assert!(permission.can_write(1000, 9999));
        assert!(permission.can_execute(1000, 9999));

        assert!(permission.can_read(9999, 2000));
        assert!(!permission.can_write(9999, 2000));
        assert!(permission.can_execute(9999, 2000));

        assert!(permission.can_read(9999, 9998));
        assert!(!permission.can_write(9999, 9998));
        assert!(!permission.can_execute(9999, 9998));
    }

    #[test]
    fn default_permission_factories_match_expected_modes() {
        assert_eq!(Permission::account_root(1, 2).mode, 0o750);
        assert_eq!(Permission::default_dir(1, 2).mode, 0o750);
        assert_eq!(Permission::default_file(1, 2).mode, 0o640);
    }
}
