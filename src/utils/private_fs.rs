//! Owner-only counterparts of `std::fs::create_dir_all` and `std::fs::write`
//! for the files glab-tui keeps on disk. Cached issues, MRs, labels and member
//! lists, and config files holding custom shell-command bindings, can carry
//! private repository data, so they must not inherit a permissive umask.
//! Outside Unix these are the plain `std::fs` calls.

use std::io;
use std::path::Path;

#[cfg(unix)]
const PRIVATE_DIR_MODE: u32 = 0o700;
#[cfg(unix)]
const PRIVATE_FILE_MODE: u32 = 0o600;

/// Creates `path` and any missing parents with mode `0700`. Directories that
/// already exist keep their mode: they may be user-chosen, such as the parent
/// of a `GLAB_TUI_CONFIG` file.
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(PRIVATE_DIR_MODE)
            .create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)
    }
}

/// Writes `contents` to `path` with mode `0600`, tightening an existing file
/// before the new contents land in it.
pub fn write(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(PRIVATE_FILE_MODE)
            .open(path)?;
        file.set_permissions(std::fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
        file.write_all(contents.as_ref())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents)
    }
}

#[cfg(all(test, unix))]
pub(crate) fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .expect("path exists")
        .permissions()
        .mode()
        & 0o777
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn every_created_directory_is_owner_only() {
        let root = tempfile::tempdir().unwrap();
        let outer = root.path().join("outer");
        let inner = outer.join("inner");

        create_dir_all(&inner).unwrap();

        assert_eq!(mode_of(&outer), 0o700);
        assert_eq!(mode_of(&inner), 0o700);
    }

    #[test]
    fn existing_directory_keeps_its_mode() {
        let root = tempfile::tempdir().unwrap();
        let existing = root.path().join("existing");
        std::fs::create_dir(&existing).unwrap();
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o755)).unwrap();

        create_dir_all(&existing).unwrap();

        assert_eq!(mode_of(&existing), 0o755);
    }

    #[test]
    fn new_file_is_owner_only() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("new.json");

        write(&file, "[]").unwrap();

        assert_eq!(mode_of(&file), 0o600);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "[]");
    }

    #[test]
    fn world_readable_file_is_tightened_and_replaced() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("old.json");
        std::fs::write(&file, "previous contents that are longer").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

        write(&file, "{}").unwrap();

        assert_eq!(mode_of(&file), 0o600);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{}");
    }
}
