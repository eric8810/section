use section_core::permission::Permission;

pub fn ensure_open_allowed(
    permission: &Permission,
    uid: u32,
    gid: u32,
    flags: i32,
) -> Result<(), i32> {
    let access_mode = flags & libc::O_ACCMODE;

    if access_mode == libc::O_RDONLY {
        if permission.can_read(uid, gid) {
            return Ok(());
        }
    } else if access_mode == libc::O_WRONLY {
        if permission.can_write(uid, gid) {
            return Ok(());
        }
    } else if access_mode == libc::O_RDWR {
        if permission.can_read(uid, gid) && permission.can_write(uid, gid) {
            return Ok(());
        }
    } else {
        return Ok(());
    }

    Err(libc::EACCES)
}

pub fn ensure_directory_write_allowed(
    permission: &Permission,
    uid: u32,
    gid: u32,
) -> Result<(), i32> {
    if permission.can_write(uid, gid) {
        Ok(())
    } else {
        Err(libc::EACCES)
    }
}

#[cfg(test)]
mod tests {
    use super::{ensure_directory_write_allowed, ensure_open_allowed};
    use section_core::permission::Permission;

    #[test]
    fn open_checks_enforce_requested_access_mode() {
        let permission = Permission::new(1000, 1000, 0o640);

        assert!(ensure_open_allowed(&permission, 1000, 1000, libc::O_RDONLY).is_ok());
        assert!(ensure_open_allowed(&permission, 1000, 1000, libc::O_WRONLY).is_ok());
        assert!(ensure_open_allowed(&permission, 1000, 1000, libc::O_RDWR).is_ok());
        assert_eq!(
            ensure_open_allowed(&permission, 2000, 2000, libc::O_WRONLY),
            Err(libc::EACCES)
        );
    }

    #[test]
    fn directory_write_check_reuses_permission_policy() {
        let permission = Permission::new(1000, 1000, 0o750);
        assert!(ensure_directory_write_allowed(&permission, 1000, 1000).is_ok());
        assert_eq!(
            ensure_directory_write_allowed(&permission, 2000, 2000),
            Err(libc::EACCES)
        );
    }
}
