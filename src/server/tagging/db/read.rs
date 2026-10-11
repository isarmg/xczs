use super::*;

impl Database {
    /// Read only the bounded visible page and never inherit tags by path alone.
    pub fn listed_tags(&self, samples: &[Option<Sample>]) -> Result<Vec<ListedTags>> {
        anyhow::ensure!(samples.len() <= 500, "tag lookup page is too large");
        database_operation!(self, 3, connection, {
            let mut result = Vec::with_capacity(samples.len());
            for sample in samples {
                let mut value = ListedTags::default();
                if let Some(sample) = sample {
                    let id: Option<i64> = sqlx::query_scalar(
                        "SELECT id FROM files WHERE root_id='main' AND path=? AND status IN ('present','suspect') AND dev=? AND ino=? AND size=? AND mtime_ns=? AND ctime_ns=? ORDER BY id DESC LIMIT 1",
                    ).bind(&sample.path).bind(sample.dev).bind(sample.ino).bind(sample.size)
                        .bind(sample.mtime_ns).bind(sample.ctime_ns).fetch_optional(&mut *connection).await?;
                    if let Some(id) = id {
                        value.file_id = Some(id);
                        value.tags = sqlx::query(
                            "SELECT t.id,CASE WHEN length(CAST(t.name AS BLOB))<=320 THEN t.name END,CASE WHEN t.color IS NULL OR length(CAST(t.color AS BLOB))<=7 THEN t.color END FROM tags t JOIN file_tags ft ON ft.tag_id=t.id WHERE ft.file_id=? ORDER BY t.name COLLATE NOCASE,t.name,t.id LIMIT 51",
                        ).bind(id).fetch_all(&mut *connection).await?.into_iter().map(|row| {
                            Ok(TagLabel { id: row.try_get(0)?, name: row.try_get(1)?, color: row.try_get(2)? })
                        }).collect::<Result<Vec<_>>>()?;
                        value.tags_has_more = value.tags.len() > PAGE_SIZE;
                        value.tags.truncate(PAGE_SIZE);
                    }
                }
                result.push(value);
            }
            Ok(result)
        })
    }

    pub fn matching_tag_paths(&self, filter: &FileQuery) -> Result<TagMatches> {
        use futures_util::TryStreamExt as _;
        database_operation!(self, 3, connection, {
            let mut q = QueryBuilder::<Sqlite>::new(
                "SELECT CASE WHEN length(CAST(physical.path AS BLOB))<=4096 THEN physical.path END,physical.dev,physical.ino,physical.size,physical.mtime_ns,physical.ctime_ns,EXISTS(SELECT 1 FROM files f",
            );
            file_filter(&mut q, filter)?;
            q.push(" AND f.id=physical.id) FROM files physical WHERE physical.root_id='main' AND physical.status IN ('present','suspect')");
            if filter.scope == "current" {
                q.push(" AND physical.parent=").push_bind(&filter.directory);
            } else if !filter.directory.is_empty() {
                // LIKE narrows candidates; the binary prefix preserves filesystem case.
                q.push(" AND (physical.parent=")
                    .push_bind(&filter.directory)
                    .push(" OR physical.path LIKE ")
                    .push_bind(format!("{}/%", escape_like(&filter.directory)))
                    .push(" ESCAPE '\\' AND substr(physical.path,1,")
                    .push_bind(filter.directory.chars().count() as i64 + 1)
                    .push(")=")
                    .push_bind(format!("{}/", filter.directory))
                    .push(" COLLATE BINARY)");
            }
            q.push(" LIMIT 100001");
            let query = q.build();
            let mut rows = query.fetch(connection);
            let mut result = std::collections::HashMap::new();
            let mut path_bytes = 0usize;
            while let Some(row) = rows.try_next().await? {
                let path: String = row.try_get(0)?;
                path_bytes = path_bytes.saturating_add(path.capacity());
                anyhow::ensure!(
                    result.len() < 100000
                        && path_bytes.saturating_add((result.len() + 1).saturating_mul(512))
                            <= 32 * 1024 * 1024,
                    "tag filter memory budget exceeded"
                );
                result.try_reserve(1)?;
                result.insert(
                    path,
                    (
                        [
                            row.try_get(1)?,
                            row.try_get(2)?,
                            row.try_get(3)?,
                            row.try_get(4)?,
                            row.try_get(5)?,
                        ],
                        row.try_get(6)?,
                    ),
                );
            }
            Ok(result)
        })
    }
    pub fn status(&self) -> Result<Status> {
        database_operation!(self, 3, connection, {
            let (root, available, last_scan_at, last_error, error_bounded): (
                String,
                i64,
                Option<i64>,
                Option<String>,
                bool,
            ) = sqlx::query_as(
                "SELECT CASE WHEN length(CAST(path AS BLOB))<=4096 THEN path END,available,last_scan_at,CASE WHEN length(CAST(last_error AS BLOB))<=1024 THEN last_error END,last_error IS NULL OR length(CAST(last_error AS BLOB))<=1024 FROM roots WHERE id='main'",
            )
            .fetch_one(&mut *connection)
            .await?;
            anyhow::ensure!(error_bounded, "persistent tag diagnostic exceeds its bound");
            let (indexed,missing,suspect): (i64,i64,i64) = sqlx::query_as(
                "SELECT COALESCE(SUM(status='present'),0),COALESCE(SUM(status='missing'),0),COALESCE(SUM(status='suspect'),0) FROM files")
                .fetch_one(connection).await?;
            Ok(Status {
                root,
                available: available != 0,
                last_scan_at,
                last_error,
                indexed,
                missing,
                suspect,
            })
        })
    }
    pub fn list(&self, query: &FileQuery) -> Result<Page> {
        database_operation!(self, 3, connection, {
            let mut count = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM files f");
            file_filter(&mut count, query)?;
            let total = count
                .build_query_scalar()
                .fetch_one(&mut *connection)
                .await?;
            let mut page = QueryBuilder::<Sqlite>::new(
                "SELECT f.id,CASE WHEN length(CAST(f.path AS BLOB))<=4096 THEN f.path END,CASE WHEN length(CAST(f.name AS BLOB))<=768 THEN f.name END,f.size,f.mtime_ns,CASE WHEN f.status IN ('present','suspect','missing') THEN f.status END FROM files f",
            );
            file_filter(&mut page, query)?;
            page.push(" ORDER BY f.path COLLATE NOCASE,f.id DESC LIMIT ")
                .push_bind(i64::from(query.page_size))
                .push(" OFFSET ")
                .push_bind(i64::from(query.page.saturating_sub(1)) * i64::from(query.page_size));
            let rows = page.build().fetch_all(&mut *connection).await?;
            let mut files = Vec::with_capacity(rows.len());
            for row in rows {
                let id = row.try_get(0)?;
                let mut tag_ids: Vec<i64> = sqlx::query_scalar(
                    "SELECT tag_id FROM file_tags WHERE file_id=? ORDER BY tag_id LIMIT 51",
                )
                .bind(id)
                .fetch_all(&mut *connection)
                .await?;
                let tag_ids_has_more = tag_ids.len() > PAGE_SIZE;
                tag_ids.truncate(PAGE_SIZE);
                files.push(FileRow {
                    id,
                    path: row.try_get(1)?,
                    name: row.try_get(2)?,
                    size: row.try_get(3)?,
                    mtime_ns: row.try_get(4)?,
                    status: row.try_get(5)?,
                    tag_ids,
                    tag_ids_has_more,
                });
            }
            Ok(Page {
                files,
                total,
                page: query.page,
                page_size: query.page_size,
            })
        })
    }
    pub fn tags(&self, cursor: Option<&str>) -> Result<TagPage> {
        let scope = pagination::scope(&[
            "tags",
            self.path.to_str().context("tag path must be UTF-8")?,
        ]);
        let cursor = Cursor::decode(cursor, &scope)?;
        database_operation!(self, 3, connection, {
            tag_page(connection, &scope, cursor, None).await
        })
    }
    pub fn file_tags_page(&self, id: i64, cursor: Option<&str>) -> Result<TagPage> {
        let scope = pagination::scope(&[
            "file-tags",
            self.path.to_str().context("tag path must be UTF-8")?,
            &id.to_string(),
        ]);
        let cursor = Cursor::decode(cursor, &scope)?;
        database_operation!(self, 3, connection, {
            tag_page(connection, &scope, cursor, Some(id)).await
        })
    }
    pub fn folders(&self, parent: &str, cursor: Option<&str>) -> Result<FolderPage> {
        let scope = pagination::scope(&[
            "folders",
            self.path.to_str().context("tag path must be UTF-8")?,
            parent,
        ]);
        let cursor = Cursor::decode(cursor, &scope)?;
        let prefix = if parent.is_empty() {
            String::new()
        } else {
            format!("{parent}/")
        };
        database_operation!(self, 3, connection, {
            const FOLDERS: &str = "WITH folder_names AS (SELECT DISTINCT substr(substr(path,?+1),1,instr(substr(path,?+1),'/')-1) AS name FROM files WHERE root_id='main' AND status IN ('present','suspect') AND path LIKE ? ESCAPE '\\' AND instr(substr(path,?+1),'/')>0 AND substr(path,1,?)=? COLLATE BINARY) ";
            let make = || {
                let mut q = QueryBuilder::<Sqlite>::new(FOLDERS);
                q.push("SELECT CASE WHEN length(CAST(name AS BLOB))<=768 THEN name END FROM folder_names");
                q
            };
            let mut q = make();
            if let Some(c) = &cursor {
                q.push(if c.reverse {
                    " WHERE name < "
                } else {
                    " WHERE name > "
                })
                .push_bind(&c.name);
            }
            q.push(if cursor.as_ref().is_some_and(|c| c.reverse) {
                " ORDER BY name DESC LIMIT 51"
            } else {
                " ORDER BY name ASC LIMIT 51"
            });
            // The fixed CTE has six leading parameters; append cursor arguments after them.
            let sql = q.sql().to_owned();
            let mut query = sqlx::query_scalar(sql)
                .bind(prefix.chars().count() as i64)
                .bind(prefix.chars().count() as i64)
                .bind(format!("{}%", escape_like(&prefix)))
                .bind(prefix.chars().count() as i64)
                .bind(prefix.chars().count() as i64)
                .bind(&prefix);
            if let Some(c) = &cursor {
                query = query.bind(&c.name);
            }
            let mut folders: Vec<String> = query.fetch_all(&mut *connection).await?;
            folders.truncate(PAGE_SIZE);
            if cursor.as_ref().is_some_and(|c| c.reverse) {
                folders.reverse();
            }
            let exists = |name: String, reverse: bool| {
                let sql = format!(
                    "{FOLDERS}SELECT EXISTS(SELECT 1 FROM folder_names WHERE name {} ?)",
                    if reverse { "<" } else { ">" }
                );
                sqlx::query_scalar::<_, bool>(sqlx::AssertSqlSafe(sql))
                    .bind(prefix.chars().count() as i64)
                    .bind(prefix.chars().count() as i64)
                    .bind(format!("{}%", escape_like(&prefix)))
                    .bind(prefix.chars().count() as i64)
                    .bind(prefix.chars().count() as i64)
                    .bind(&prefix)
                    .bind(name)
            };
            let previous_cursor = if let Some(name) = folders.first() {
                if exists(name.clone(), true)
                    .fetch_one(&mut *connection)
                    .await?
                {
                    Some(Cursor::at(&scope, name, 0, true)?)
                } else {
                    None
                }
            } else {
                None
            };
            let next_cursor = if let Some(name) = folders.last() {
                if exists(name.clone(), false)
                    .fetch_one(&mut *connection)
                    .await?
                {
                    Some(Cursor::at(&scope, name, 0, false)?)
                } else {
                    None
                }
            } else {
                None
            };
            Ok(FolderPage {
                folders,
                previous_cursor,
                next_cursor,
            })
        })
    }
    #[cfg(test)]
    pub fn download_record(&self, id: i64) -> Result<Option<Sample>> {
        database_operation!(self, 3, connection, {
            Ok(sqlx::query("SELECT path,parent,name,dev,ino,size,mtime_ns,ctime_ns FROM files WHERE id=? AND status IN ('present','suspect')")
                .bind(id).try_map(|r|Ok(Sample {path:r.try_get(0)?,parent:r.try_get(1)?,name:r.try_get(2)?,dev:r.try_get(3)?,ino:r.try_get(4)?,size:r.try_get(5)?,mtime_ns:r.try_get(6)?,ctime_ns:r.try_get(7)?}))
                .fetch_optional(connection).await?)
        })
    }
}

async fn tag_page(
    connection: &mut SqliteConnection,
    scope: &str,
    cursor: Option<Cursor>,
    file: Option<i64>,
) -> Result<TagPage> {
    let mut q = QueryBuilder::<Sqlite>::new(
        "SELECT t.id,CASE WHEN length(CAST(t.name AS BLOB))<=320 THEN t.name END,CASE WHEN t.color IS NULL OR length(CAST(t.color AS BLOB))<=7 THEN t.color END,(SELECT COUNT(*) FROM file_tags ft WHERE ft.tag_id=t.id),t.color IS NULL OR length(CAST(t.color AS BLOB))<=7 FROM tags t WHERE 1=1",
    );
    if let Some(id) = file {
        q.push(" AND EXISTS(SELECT 1 FROM file_tags ft WHERE ft.tag_id=t.id AND ft.file_id=")
            .push_bind(id)
            .push(")");
    }
    if let Some(c) = &cursor {
        q.push(if c.reverse {
            " AND (t.name COLLATE NOCASE,t.name,t.id)<("
        } else {
            " AND (t.name COLLATE NOCASE,t.name,t.id)>("
        })
        .push_bind(&c.name)
        .push(",")
        .push_bind(&c.name)
        .push(",")
        .push_bind(c.id)
        .push(")");
    }
    q.push(if cursor.as_ref().is_some_and(|c| c.reverse) {
        " ORDER BY t.name COLLATE NOCASE DESC,t.name DESC,t.id DESC LIMIT 51"
    } else {
        " ORDER BY t.name COLLATE NOCASE,t.name,t.id LIMIT 51"
    });
    let mut tags = q
        .build()
        .try_map(|r| {
            if !r.try_get::<bool, _>(4)? {
                return Err(sqlx::Error::Decode(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "persistent tag color exceeds its bound",
                ))));
            }
            Ok(TagRow {
                id: r.try_get(0)?,
                name: r.try_get(1)?,
                color: r.try_get(2)?,
                file_count: r.try_get(3)?,
            })
        })
        .fetch_all(&mut *connection)
        .await?;
    tags.truncate(PAGE_SIZE);
    if cursor.as_ref().is_some_and(|c| c.reverse) {
        tags.reverse();
    }
    let mut cursors = [None, None];
    for (i, row) in [tags.first(), tags.last()].into_iter().enumerate() {
        if let Some(row) = row {
            let mut q = QueryBuilder::<Sqlite>::new("SELECT EXISTS(SELECT 1 FROM tags t WHERE ");
            q.push(if i == 0 {
                "(t.name COLLATE NOCASE,t.name,t.id)<("
            } else {
                "(t.name COLLATE NOCASE,t.name,t.id)>("
            })
            .push_bind(&row.name)
            .push(",")
            .push_bind(&row.name)
            .push(",")
            .push_bind(row.id)
            .push(")");
            if let Some(id) = file {
                q.push(
                    " AND EXISTS(SELECT 1 FROM file_tags ft WHERE ft.tag_id=t.id AND ft.file_id=",
                )
                .push_bind(id)
                .push(")");
            }
            q.push(")");
            if q.build_query_scalar::<bool>()
                .fetch_one(&mut *connection)
                .await?
            {
                cursors[i] = Some(Cursor::at(scope, &row.name, row.id, i == 0)?);
            }
        }
    }
    Ok(TagPage {
        tags,
        previous_cursor: cursors[0].take(),
        next_cursor: cursors[1].take(),
    })
}

fn file_filter(q: &mut QueryBuilder<Sqlite>, filter: &FileQuery) -> Result<()> {
    q.push(" WHERE f.root_id='main'");
    if filter.status != "all" {
        q.push(" AND f.status=").push_bind(&filter.status);
    } else {
        q.push(" AND f.status<>'relinked'");
    }
    if !filter.search.is_empty() {
        q.push(" AND f.name LIKE ")
            .push_bind(format!("%{}%", escape_like(&filter.search)))
            .push(" ESCAPE '\\'");
    }
    match filter.scope.as_str() {
        "current" => {
            q.push(" AND f.parent=").push_bind(&filter.directory);
        }
        "recursive" if !filter.directory.is_empty() => {
            // Filename search is case-insensitive, but directory identity is not.
            q.push(" AND (f.parent=")
                .push_bind(&filter.directory)
                .push(" OR f.path LIKE ")
                .push_bind(format!("{}/%", escape_like(&filter.directory)))
                .push(" ESCAPE '\\' AND substr(f.path,1,")
                .push_bind(filter.directory.chars().count() as i64 + 1)
                .push(")=")
                .push_bind(format!("{}/", filter.directory))
                .push(" COLLATE BINARY)");
        }
        "all" | "recursive" => {}
        _ => bail!("invalid scope"),
    }
    for id in &filter.all_tags {
        q.push(" AND EXISTS(SELECT 1 FROM file_tags ft WHERE ft.file_id=f.id AND ft.tag_id=")
            .push_bind(*id)
            .push(")");
    }
    for (ids, exclude) in [(&filter.any_tags, false), (&filter.exclude_tags, true)] {
        if ids.is_empty() {
            continue;
        }
        q.push(if exclude {
            " AND NOT EXISTS(SELECT 1 FROM file_tags ft WHERE ft.file_id=f.id AND ft.tag_id IN ("
        } else {
            " AND EXISTS(SELECT 1 FROM file_tags ft WHERE ft.file_id=f.id AND ft.tag_id IN ("
        });
        for (i, id) in ids.iter().enumerate() {
            if i != 0 {
                q.push(",");
            }
            q.push_bind(*id);
        }
        q.push("))");
    }
    Ok(())
}
