//! Indexed file tags stored outside the served filesystem.
pub(super) mod api;
mod capacity;
pub(super) mod db;
mod pagination;
mod scan;

use anyhow::{Result, bail};
use db::Database;
use std::{path::PathBuf, sync::Arc};
use xcss::fs_safety::linux::{FileIdentity, MountPolicy, OpenAt2Root};

pub(super) struct Tagging {
    pub(super) db: Arc<Database>,
    pub(super) state_dir: PathBuf,
    root_path: PathBuf,
    root_identity: FileIdentity,
    scan_lock: Arc<tokio::sync::Mutex<()>>,
    max_entries: usize,
}

impl Tagging {
    pub(super) fn new(
        state_dir: PathBuf,
        root_path: PathBuf,
        root_identity: (u64, u64),
        max_entries: usize,
        prepared: db::PreparedDatabase,
    ) -> Result<Self> {
        let db = Arc::new(prepared.open()?);
        Ok(Self {
            db,
            state_dir,
            root_path,
            root_identity: FileIdentity {
                device: root_identity.0,
                inode: root_identity.1,
            },
            scan_lock: Arc::new(tokio::sync::Mutex::new(())),
            max_entries,
        })
    }

    pub(super) fn live_root(&self) -> Result<OpenAt2Root> {
        let root = OpenAt2Root::open(&self.root_path, MountPolicy::SameMount)?;
        if root.identity() != self.root_identity {
            bail!("configured file root identity changed");
        }
        scan::assert_root(&self.root_path, self.root_identity)?;
        Ok(root)
    }

    pub(super) async fn scan_once(self: &Arc<Self>) -> Result<usize> {
        let guard = self
            .scan_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| anyhow::anyhow!("a tag scan is already active"))?;
        let tagging = Arc::clone(self);

        tokio::task::spawn_blocking(move || {
            // Cancellation of an HTTP waiter cannot admit a second scan while
            // the actual filesystem scan/database transaction is still active.
            let _guard = guard;
            let result = (|| {
                let root = tagging.live_root()?;
                let samples = scan::collect(&root, tagging.max_entries)?;
                scan::assert_root(&tagging.root_path, tagging.root_identity)?;
                let count = samples.len();
                tagging.db.scan_complete(samples)?;
                Ok::<_, anyhow::Error>(count)
            })();
            if result.is_err() {
                // Record the actual worker's failure even when the HTTP waiter was cancelled.
                // Keep diagnostics fixed and bounded, rather than storing arbitrary paths/errors.
                if tagging.db.scan_error("tag_scan_failed").is_err() {
                    log::error!("The tag scan failure could not be recorded");
                }
            }
            result
        })
        .await?
    }
}
