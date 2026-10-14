use super::db::Sample;
use crate::server::internal_names::is_internal_name;
use anyhow::{Context, Result, bail};
use rustix::{
    fs::{Dir, FileType, Mode, OFlags, ResolveFlags, fstat, openat2},
    io::Errno,
};
use std::mem::size_of;
use std::{
    ffi::OsString,
    fs,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};
use xcss::fs_safety::linux::{FileIdentity, OpenAt2Root};

const RESOLVE: ResolveFlags = ResolveFlags::BENEATH
    .union(ResolveFlags::NO_SYMLINKS)
    .union(ResolveFlags::NO_XDEV);

pub fn assert_root(root_path: &Path, identity: FileIdentity) -> Result<()> {
    let metadata = fs::symlink_metadata(root_path)
        .with_context(|| format!("root {} unavailable", root_path.display()))?;
    if !metadata.is_dir() || FileIdentity::from_metadata(&metadata) != identity {
        bail!("configured root identity changed or disappeared");
    }
    Ok(())
}

pub const MAX_SCAN_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
#[error("tag scan buffer capacity is exhausted")]
pub struct ScanCapacity;

fn push_budgeted<T>(
    values: &mut Vec<T>,
    value: T,
    heap_bytes: usize,
    retained_bytes: &mut usize,
    max_bytes: usize,
) -> Result<()> {
    let old_capacity = values.capacity();
    let extra = if values.len() == old_capacity {
        old_capacity.max(8)
    } else {
        0
    };
    let next = retained_bytes
        .checked_add(heap_bytes)
        .and_then(|v| v.checked_add(extra.checked_mul(size_of::<T>())?))
        .ok_or(ScanCapacity)?;
    if next > max_bytes {
        return Err(ScanCapacity.into());
    }
    if extra != 0 {
        values.try_reserve_exact(extra)?;
    }
    let next = retained_bytes
        .checked_add(heap_bytes)
        .and_then(|v| {
            v.checked_add((values.capacity() - old_capacity).checked_mul(size_of::<T>())?)
        })
        .ok_or(ScanCapacity)?;
    if next > max_bytes {
        return Err(ScanCapacity.into());
    }
    *retained_bytes = next;
    values.push(value);
    Ok(())
}

pub fn collect(root: &OpenAt2Root, max_entries: usize) -> Result<Vec<Sample>> {
    collect_with_budget(root, max_entries, MAX_SCAN_BYTES)
}

fn collect_with_budget(
    root: &OpenAt2Root,
    max_entries: usize,
    max_bytes: usize,
) -> Result<Vec<Sample>> {
    let mut samples = Vec::new();
    let mut stack = Vec::new();
    let mut retained_bytes = 0;
    push_budgeted(
        &mut stack,
        PathBuf::new(),
        0,
        &mut retained_bytes,
        max_bytes,
    )?;
    let mut seen = 0usize;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while let Some(dir_path) = stack.pop() {
        if std::time::Instant::now() >= deadline {
            return Err(ScanCapacity.into());
        }
        let current_directory_bytes = dir_path.capacity();
        let relative = if dir_path.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir_path.as_path()
        };
        let fd = openat2(
            root.directory(),
            relative,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
            RESOLVE,
        )
        .with_context(|| format!("open directory {}", relative.display()))?;
        let mut dir = Dir::new(fd)?;
        while let Some(item) = dir.read() {
            if std::time::Instant::now() >= deadline {
                return Err(ScanCapacity.into());
            }
            let entry = item?;
            let name = entry.file_name().to_bytes();
            if matches!(name, b"." | b"..") {
                continue;
            }
            let name = OsString::from_vec(name.to_vec());
            if name.to_str().is_some_and(is_internal_name) {
                continue;
            }
            seen += 1;
            if seen > max_entries {
                bail!("entry count exceeded scan limit");
            }
            let child = dir_path.join(name);
            let fd = match openat2(
                root.directory(),
                &child,
                OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
                RESOLVE,
            ) {
                Ok(fd) => fd,
                Err(Errno::LOOP | Errno::XDEV) => continue,
                Err(error) => {
                    return Err(error).with_context(|| format!("inspect {}", child.display()));
                }
            };
            let stat = fstat(fd)?;
            match FileType::from_raw_mode(stat.st_mode) {
                FileType::Directory => {
                    let bytes = child.capacity();
                    push_budgeted(&mut stack, child, bytes, &mut retained_bytes, max_bytes)?;
                }
                FileType::RegularFile => {
                    let Some(path) = child.to_str() else {
                        continue;
                    };
                    let Some(name) = child.file_name().and_then(|name| name.to_str()) else {
                        continue;
                    };
                    let parent = dir_path.to_str().context("parent path is not UTF-8")?;
                    let sample = Sample {
                        path: path.into(),
                        parent: parent.into(),
                        name: name.into(),
                        dev: stat.st_dev as i64,
                        ino: stat.st_ino as i64,
                        size: stat.st_size,
                        mtime_ns: stat
                            .st_mtime
                            .checked_mul(1_000_000_000)
                            .and_then(|seconds| seconds.checked_add(stat.st_mtime_nsec as i64))
                            .context("file modification time exceeds the index range")?,
                        ctime_ns: stat
                            .st_ctime
                            .checked_mul(1_000_000_000)
                            .and_then(|seconds| seconds.checked_add(stat.st_ctime_nsec as i64))
                            .context("file change time exceeds the index range")?,
                    };
                    let bytes =
                        sample.path.capacity() + sample.parent.capacity() + sample.name.capacity();
                    push_budgeted(&mut samples, sample, bytes, &mut retained_bytes, max_bytes)?;
                }
                _ => {}
            }
        }
        // Keep accounting for this directory while its path is still in use.
        // The vector buffer stays allocated when the path itself is released.
        retained_bytes -= current_directory_bytes;
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xcss::fs_safety::linux::MountPolicy;
    #[test]
    fn scan_refuses_retained_path_bytes_before_returning_partial_samples() {
        let directory = tempfile::tempdir().unwrap();
        for index in 0..40 {
            fs::create_dir(
                directory
                    .path()
                    .join(format!("folder-{index:03}-{}", "x".repeat(100))),
            )
            .unwrap();
        }
        let root = OpenAt2Root::open(directory.path(), MountPolicy::SameMount).unwrap();
        assert!(
            collect_with_budget(&root, 100, 2048)
                .unwrap_err()
                .is::<ScanCapacity>()
        );
        // All source entries remain intact after admission failure.
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 40);
        assert!(
            collect_with_budget(&root, 100, 16 * 1024)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn scan_rejects_unrepresentable_file_time_without_panicking() {
        use std::time::{Duration, UNIX_EPOCH};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("future.txt");
        let file = fs::File::create(&path).unwrap();
        // A legal filesystem timestamp beyond the signed nanosecond range.
        let modified = UNIX_EPOCH + Duration::from_secs(10_000_000_000);
        file.set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        let root = OpenAt2Root::open(directory.path(), MountPolicy::SameMount).unwrap();
        let result = collect(&root, 10);
        assert!(
            result.is_err(),
            "an out-of-range identity must not be indexed"
        );
        assert!(path.exists());
        file.set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH))
            .unwrap();
        assert_eq!(collect(&root, 10).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn failed_time_scan_retains_index_and_records_recoverable_failure() {
        use crate::{
            Args,
            server::{Server, ServerLifecycle},
        };
        use std::{
            os::unix::fs::PermissionsExt,
            sync::Arc,
            time::{Duration, UNIX_EPOCH},
        };
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        fs::set_permissions(state.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let server = Server::initialize_with_lifecycle_for_test(
            Args {
                serve_path: root.path().into(),
                root: state.path().into(),
                state_dir: Some(state.path().join("state")),
                ..Args::default()
            },
            ServerLifecycle::new(),
        )
        .unwrap();
        fs::write(root.path().join("kept.txt"), b"kept").unwrap();
        assert_eq!(server.tagging.scan_once().await.unwrap(), 1);
        let tagging = Arc::clone(&server.tagging);
        let before = tagging.db.status().unwrap();
        assert!(before.available);
        let file = fs::File::create(root.path().join("future.txt")).unwrap();
        file.set_times(
            fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(10_000_000_000)),
        )
        .unwrap();
        assert!(tagging.scan_once().await.is_err());
        let failed = tagging.db.status().unwrap();
        assert!(!failed.available);
        assert_eq!(failed.last_error.as_deref(), Some("tag_scan_failed"));
        assert_eq!(failed.indexed, before.indexed);
        assert_eq!(failed.missing, before.missing);
        file.set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH))
            .unwrap();
        assert_eq!(tagging.scan_once().await.unwrap(), 2);
        let recovered = tagging.db.status().unwrap();
        assert!(recovered.available);
        assert!(recovered.last_error.is_none());
    }

    #[test]
    fn scan_skips_symlinks_and_never_reads_outside_root() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join("folder")).unwrap();
        fs::write(t.path().join("folder/good"), b"a").unwrap();
        std::os::unix::fs::symlink("/etc", t.path().join("escape")).unwrap();
        let root = OpenAt2Root::open(t.path(), MountPolicy::SameMount).unwrap();
        let files = collect(&root, 10).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "folder/good");
    }
}
