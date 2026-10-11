use super::*;

impl Database {
    pub fn scan_complete(&self, samples: Vec<Sample>) -> Result<()> {
        database_operation!(self, 30, connection, {
            let mut tx = connection.begin_with("BEGIN IMMEDIATE").await?;
            let generation: i64 =
                sqlx::query_scalar("SELECT scan_generation+1 FROM roots WHERE id='main'")
                    .fetch_one(&mut *tx)
                    .await?;
            let mut retained: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM files")
                .fetch_one(&mut *tx)
                .await?;
            for sample in samples {
                let previous: Option<(i64,i64,i64,i64,i64,i64)> = sqlx::query_as(
                    "SELECT id,dev,ino,size,mtime_ns,ctime_ns FROM files WHERE root_id='main' AND path=? AND status IN ('present','suspect') ORDER BY id DESC LIMIT 1")
                    .bind(&sample.path).fetch_optional(&mut *tx).await?;
                let was_known: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM files WHERE root_id='main' AND path=?)",
                )
                .bind(&sample.path)
                .fetch_one(&mut *tx)
                .await?;
                if let Some((id, dev, ino, size, mtime, ctime)) = previous {
                    if (dev, ino, size, mtime, ctime)
                        == (
                            sample.dev,
                            sample.ino,
                            sample.size,
                            sample.mtime_ns,
                            sample.ctime_ns,
                        )
                    {
                        sqlx::query("UPDATE files SET last_seen_generation=? WHERE id=?")
                            .bind(generation)
                            .bind(id)
                            .execute(&mut *tx)
                            .await?;
                        continue;
                    }
                    sqlx::query("UPDATE files SET status='missing' WHERE id=?")
                        .bind(id)
                        .execute(&mut *tx)
                        .await?;
                }
                if retained >= self.limits.files {
                    return Err(Exhausted.into());
                }
                let bytes =
                    (sample.path.len() + sample.parent.len() + sample.name.len() + 512) as u64;
                capacity::storage(&mut tx, &self.path, bytes.saturating_mul(2), self.limits)
                    .await?;
                sqlx::query("INSERT INTO files(root_id,path,parent,name,dev,ino,size,mtime_ns,ctime_ns,status,last_seen_generation) VALUES('main',?,?,?,?,?,?,?,?,?,?)")
                    .bind(sample.path).bind(sample.parent).bind(sample.name).bind(sample.dev).bind(sample.ino).bind(sample.size)
                    .bind(sample.mtime_ns).bind(sample.ctime_ns).bind(if was_known {"suspect"} else {"present"}).bind(generation)
                    .execute(&mut *tx).await?;
                retained += 1;
            }
            sqlx::query("UPDATE files SET status='missing' WHERE root_id='main' AND status IN ('present','suspect') AND last_seen_generation<>?")
                .bind(generation).execute(&mut *tx).await?;
            sqlx::query("UPDATE roots SET available=1,scan_generation=?,last_scan_at=?,last_error=NULL WHERE id='main'")
                .bind(generation).bind(now()?).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok(())
        })
    }
    pub fn scan_error(&self, message: &str) -> Result<()> {
        database_operation!(self, 3, connection, {
            sqlx::query("UPDATE roots SET available=0,last_error=? WHERE id='main'")
                .bind(message)
                .execute(connection)
                .await?;
            Ok(())
        })
    }
    pub fn create_tag(&self, name: &str, color: Option<&str>) -> Result<i64> {
        database_operation!(self, 3, connection, {
            let mut tx = connection.begin_with("BEGIN IMMEDIATE").await?;
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tags")
                .fetch_one(&mut *tx)
                .await?;
            if count >= self.limits.tags {
                return Err(Exhausted.into());
            }
            capacity::storage(&mut tx, &self.path, 2048, self.limits).await?;
            let id = sqlx::query("INSERT INTO tags(name,color) VALUES(?,?)")
                .bind(name)
                .bind(color)
                .execute(&mut *tx)
                .await?
                .last_insert_rowid();
            tx.commit().await?;
            Ok(id)
        })
    }
    pub fn update_tag(&self, id: i64, name: &str, color: Option<&str>) -> Result<bool> {
        database_operation!(self, 3, connection, {
            Ok(sqlx::query("UPDATE tags SET name=?,color=? WHERE id=?")
                .bind(name)
                .bind(color)
                .bind(id)
                .execute(connection)
                .await?
                .rows_affected()
                > 0)
        })
    }
    pub fn delete_tag(&self, id: i64) -> Result<bool> {
        database_operation!(self, 3, connection, {
            Ok(sqlx::query("DELETE FROM tags WHERE id=?")
                .bind(id)
                .execute(connection)
                .await?
                .rows_affected()
                > 0)
        })
    }
    pub fn mutate_tags(&self, file_ids: &[i64], tag_ids: &[i64], add: bool) -> Result<()> {
        database_operation!(self, 3, connection, {
            let mut tx = connection.begin_with("BEGIN IMMEDIATE").await?;
            for id in file_ids {
                let status: Option<String> =
                    sqlx::query_scalar("SELECT status FROM files WHERE id=?")
                        .bind(id)
                        .fetch_optional(&mut *tx)
                        .await?;
                anyhow::ensure!(
                    matches!(status.as_deref(), Some("present" | "suspect")),
                    "file is missing or unavailable"
                );
            }
            for id in tag_ids {
                let exists: bool =
                    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tags WHERE id=?)")
                        .bind(id)
                        .fetch_one(&mut *tx)
                        .await?;
                anyhow::ensure!(exists, "unknown tag");
            }
            let mut retained: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM file_tags")
                .fetch_one(&mut *tx)
                .await?;
            for file in file_ids {
                let mut file_count: i64 = if add {
                    sqlx::query_scalar("SELECT COUNT(*) FROM file_tags WHERE file_id=?")
                        .bind(file)
                        .fetch_one(&mut *tx)
                        .await?
                } else {
                    0
                };
                for tag in tag_ids {
                    if add {
                        let exists: bool = sqlx::query_scalar(
                            "SELECT EXISTS(SELECT 1 FROM file_tags WHERE file_id=? AND tag_id=?)",
                        )
                        .bind(file)
                        .bind(tag)
                        .fetch_one(&mut *tx)
                        .await?;
                        if exists {
                            continue;
                        }
                        if file_count >= MAX_FILE_TAGS {
                            return Err(TagLimitExceeded.into());
                        }
                        if retained >= self.limits.relations {
                            return Err(Exhausted.into());
                        }
                        capacity::storage(&mut tx, &self.path, 512, self.limits).await?;
                        sqlx::query("INSERT INTO file_tags(file_id,tag_id) VALUES(?,?)")
                            .bind(file)
                            .bind(tag)
                            .execute(&mut *tx)
                            .await?;
                        retained += 1;
                        file_count += 1;
                    } else {
                        sqlx::query("DELETE FROM file_tags WHERE file_id=? AND tag_id=?")
                            .bind(file)
                            .bind(tag)
                            .execute(&mut *tx)
                            .await?;
                    }
                }
            }
            tx.commit().await?;
            Ok(())
        })
    }
    pub fn relink(&self, old_id: i64, new_id: i64) -> Result<()> {
        anyhow::ensure!(old_id != new_id, RelinkConflict);
        database_operation!(self, 3, connection, {
            let mut tx = connection.begin_with("BEGIN IMMEDIATE").await?;
            let old: Option<String> = sqlx::query_scalar("SELECT status FROM files WHERE id=?")
                .bind(old_id)
                .fetch_optional(&mut *tx)
                .await?;
            let new: Option<String> = sqlx::query_scalar("SELECT status FROM files WHERE id=?")
                .bind(new_id)
                .fetch_optional(&mut *tx)
                .await?;
            anyhow::ensure!(
                old.as_deref() == Some("missing")
                    && matches!(new.as_deref(), Some("present" | "suspect")),
                RelinkConflict
            );
            let (current, additional): (i64, i64) = sqlx::query_as(
                "SELECT (SELECT COUNT(*) FROM file_tags WHERE file_id=?),(SELECT COUNT(*) FROM file_tags old WHERE old.file_id=? AND NOT EXISTS(SELECT 1 FROM file_tags new WHERE new.file_id=? AND new.tag_id=old.tag_id))",
            ).bind(new_id).bind(old_id).bind(new_id).fetch_one(&mut *tx).await?;
            if additional > 0 && current + additional > MAX_FILE_TAGS {
                return Err(TagLimitExceeded.into());
            }
            sqlx::query("INSERT OR IGNORE INTO file_tags(file_id,tag_id) SELECT ?,tag_id FROM file_tags WHERE file_id=?").bind(new_id).bind(old_id).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM file_tags WHERE file_id=?")
                .bind(old_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE files SET status='relinked' WHERE id=?")
                .bind(old_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE files SET status='present' WHERE id=?")
                .bind(new_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok(())
        })
    }
    pub fn confirm(&self, id: i64) -> Result<bool> {
        database_operation!(self, 3, connection, {
            Ok(
                sqlx::query("UPDATE files SET status='present' WHERE id=? AND status='suspect'")
                    .bind(id)
                    .execute(connection)
                    .await?
                    .rows_affected()
                    > 0,
            )
        })
    }
    pub fn backup(&self, path: &Path) -> Result<()> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        database_operation!(self, 30, connection, {
            let parent = path.parent().context("backup has no parent")?;
            let mut count = 0usize;
            let mut bytes = 0u64;
            for entry in std::fs::read_dir(parent)? {
                let entry = entry?;
                let metadata = std::fs::symlink_metadata(entry.path())?;
                anyhow::ensure!(
                    metadata.is_file()
                        && !metadata.file_type().is_symlink()
                        && metadata.nlink() == 1,
                    "invalid tag backup identity"
                );
                count += 1;
                if count >= 4 {
                    return Err(Exhausted.into());
                }
                bytes = bytes.checked_add(metadata.len()).ok_or(Exhausted)?;
            }
            let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
                .fetch_one(&mut *connection)
                .await?;
            let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
                .fetch_one(&mut *connection)
                .await?;
            let estimate = u64::try_from(pages)?
                .checked_mul(u64::try_from(page_size)?)
                .ok_or(Exhausted)?;
            let available = rustix::fs::statvfs(parent)?;
            if bytes.saturating_add(estimate) > 32 * 1024 * 1024 * 1024
                || available.f_bavail.saturating_mul(available.f_frsize)
                    < self
                        .limits
                        .free_floor
                        .saturating_add(estimate)
                        .saturating_add(64 * 1024 * 1024)
            {
                return Err(Exhausted.into());
            }
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)?;
            let result = sqlx::query("VACUUM INTO ?")
                .bind(path.to_str().context("backup path must be UTF-8")?)
                .execute(&mut *connection)
                .await;
            if let Err(error) = result {
                drop(file);
                let _ = std::fs::remove_file(path);
                return Err(error.into());
            }
            file.sync_all()?;
            std::fs::File::open(parent)?.sync_all()?;
            Ok(())
        })
    }
}
