use std::io;
use std::path::{Path, PathBuf};

/// Writes [`SERVICE_WORKER`](crate::SERVICE_WORKER) to `pushups-sw.js` in `dir`, for a build
/// script whose bundler copies the file into the site, and returns its path.
///
/// A file that already holds these bytes is left untouched, so a watcher such as `trunk serve`
/// sees no change and starts no rebuild.
///
/// # Errors
///
/// The error of reading the existing file, other than its absence, or of writing the new one.
pub fn write_service_worker(dir: impl AsRef<Path>) -> io::Result<PathBuf> {
    let path = dir.as_ref().join("pushups-sw.js");
    let current = match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if current.as_deref() != Some(crate::SERVICE_WORKER.as_bytes()) {
        std::fs::write(&path, crate::SERVICE_WORKER)?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn the_worker_is_written_into_the_directory() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = write_service_worker(dir.path()).expect("the worker is written");
        assert_eq!(path, dir.path().join("pushups-sw.js"));
        assert_eq!(
            fs::read_to_string(&path).expect("the worker"),
            crate::SERVICE_WORKER
        );
    }

    #[test]
    fn a_worker_with_other_bytes_is_replaced() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        fs::write(dir.path().join("pushups-sw.js"), "// an older worker").expect("a stale worker");
        let path = write_service_worker(dir.path()).expect("the worker is written");
        assert_eq!(
            fs::read_to_string(path).expect("the worker"),
            crate::SERVICE_WORKER
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_unchanged_worker_is_not_written_again() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("pushups-sw.js");
        fs::write(&path, crate::SERVICE_WORKER).expect("a current worker");
        // A write to a read-only file fails, so success means no write was attempted.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).expect("read-only");
        assert_eq!(
            write_service_worker(dir.path()).expect("nothing to write"),
            path
        );
    }

    #[test]
    fn a_missing_directory_is_an_error() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        assert!(write_service_worker(dir.path().join("absent")).is_err());
    }
}
