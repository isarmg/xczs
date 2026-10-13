use super::*;
mod business_cases {
    use super::*;
    #[test]
    fn scan_preserves_tags_and_does_not_apply_them_to_replacements() {
        let t = tempfile::Builder::new()
            .permissions(
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
            )
            .tempdir()
            .unwrap();
        Database::initialize(&t.path().join("db"), t.path()).unwrap();
        let db = Database::open(&t.path().join("db"), t.path()).unwrap();
        let sample = |ino| Sample {
            path: "a.txt".into(),
            parent: "".into(),
            name: "a.txt".into(),
            dev: 1,
            ino,
            size: 1,
            mtime_ns: 1,
            ctime_ns: 1,
        };
        db.scan_complete(vec![sample(1)]).unwrap();
        let old = db
            .list(&FileQuery {
                search: "".into(),
                directory: "".into(),
                scope: "all".into(),
                status: "all".into(),
                all_tags: vec![],
                any_tags: vec![],
                exclude_tags: vec![],
                page: 1,
                page_size: 50,
            })
            .unwrap()
            .files[0]
            .id;
        let tag = db.create_tag("important", None).unwrap();
        db.mutate_tags(&[old], &[tag], true).unwrap();
        db.scan_complete(vec![sample(2)]).unwrap();
        assert!(db.download_record(old).unwrap().is_none());
        let rows = db
            .list(&FileQuery {
                search: "".into(),
                directory: "".into(),
                scope: "all".into(),
                status: "all".into(),
                all_tags: vec![],
                any_tags: vec![],
                exclude_tags: vec![],
                page: 1,
                page_size: 50,
            })
            .unwrap()
            .files;
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter()
                .find(|r| r.status == "suspect")
                .unwrap()
                .tag_ids
                .len(),
            0
        );
        db.scan_error("mount failed").unwrap();
        assert_eq!(db.status().unwrap().missing, 1);
    }

    #[test]
    fn directory_and_tag_filters_compose_without_rescanning_files() {
        let t = tempfile::Builder::new()
            .permissions(
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
            )
            .tempdir()
            .unwrap();
        Database::initialize(&t.path().join("db"), t.path()).unwrap();
        let db = Database::open(&t.path().join("db"), t.path()).unwrap();
        let sample = |path: &str, ino| Sample {
            path: path.into(),
            parent: path
                .rsplit_once('/')
                .map_or("", |(parent, _)| parent)
                .into(),
            name: path.rsplit('/').next().unwrap().into(),
            dev: 1,
            ino,
            size: 1,
            mtime_ns: 1,
            ctime_ns: 1,
        };
        db.scan_complete(vec![
            sample("docs/a.txt", 1),
            sample("docs/sub/b.txt", 2),
            sample("other/c.txt", 3),
        ])
        .unwrap();
        let query = |scope: &str, directory: &str, all, any, exclude| FileQuery {
            search: String::new(),
            directory: directory.into(),
            scope: scope.into(),
            status: "present".into(),
            all_tags: all,
            any_tags: any,
            exclude_tags: exclude,
            page: 1,
            page_size: 50,
        };
        let rows = db.list(&query("all", "", vec![], vec![], vec![])).unwrap();
        let id = |path: &str| rows.files.iter().find(|file| file.path == path).unwrap().id;
        let first = db.create_tag("first", None).unwrap();
        let second = db.create_tag("second", None).unwrap();
        let third = db.create_tag("third", None).unwrap();
        db.mutate_tags(&[id("docs/a.txt")], &[first, second], true)
            .unwrap();
        db.mutate_tags(&[id("docs/sub/b.txt")], &[first], true)
            .unwrap();
        db.mutate_tags(&[id("other/c.txt")], &[second, third], true)
            .unwrap();
        let paths = |query| {
            db.list(&query)
                .unwrap()
                .files
                .into_iter()
                .map(|file| file.path)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            paths(query("current", "docs", vec![], vec![], vec![])),
            vec!["docs/a.txt"]
        );
        assert_eq!(
            paths(query("recursive", "docs", vec![], vec![], vec![])),
            vec!["docs/a.txt", "docs/sub/b.txt"]
        );
        assert_eq!(
            paths(query("all", "", vec![first, second], vec![], vec![])),
            vec!["docs/a.txt"]
        );
        assert_eq!(
            paths(query("all", "", vec![], vec![second, third], vec![])),
            vec!["docs/a.txt", "other/c.txt"]
        );
        assert_eq!(
            paths(query("all", "", vec![], vec![], vec![second])),
            vec!["docs/sub/b.txt"]
        );
        assert_eq!(
            paths(query(
                "all",
                "",
                vec![first],
                vec![second, third],
                vec![third]
            )),
            vec!["docs/a.txt"]
        );
    }
}

mod resource_tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Database) {
        let root = tempfile::Builder::new()
            .permissions(
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
            )
            .tempdir()
            .unwrap();
        let path = root.path().join("tags.db");
        Database::initialize(&path, root.path()).unwrap();
        let mut database = Database::open(&path, root.path()).unwrap();
        database.limits.free_floor = 0;
        (root, database)
    }
    fn sample(path: &str, ino: i64) -> Sample {
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        Sample {
            path: path.into(),
            parent: parent.into(),
            name: name.into(),
            dev: 1,
            ino,
            size: 1,
            mtime_ns: 1,
            ctime_ns: 1,
        }
    }
    fn query() -> FileQuery {
        FileQuery {
            search: String::new(),
            directory: String::new(),
            scope: "all".into(),
            status: "all".into(),
            all_tags: vec![],
            any_tags: vec![],
            exclude_tags: vec![],
            page: 1,
            page_size: 100,
        }
    }
    // Model pre-limit data without bypassing the production mutation checks.
    fn legacy_tags(db: &Database, file: i64, tags: &[i64]) -> Result<()> {
        database_operation!(db, 3, connection, {
            for tag in tags {
                sqlx::query("INSERT INTO file_tags(file_id,tag_id) VALUES(?,?)")
                    .bind(file)
                    .bind(tag)
                    .execute(&mut *connection)
                    .await?;
            }
            Ok::<_, anyhow::Error>(())
        })
    }

    #[test]
    fn sixteen_tag_limit_is_net_new_and_batch_atomic() {
        let (_root, db) = fixture();
        db.scan_complete(vec![sample("first", 1), sample("second", 2)])
            .unwrap();
        let files = db.list(&query()).unwrap().files;
        let first = files.iter().find(|f| f.path == "first").unwrap().id;
        let second = files.iter().find(|f| f.path == "second").unwrap().id;
        let tags = (0..18)
            .map(|i| db.create_tag(&format!("tag-{i:02}"), None).unwrap())
            .collect::<Vec<_>>();
        db.mutate_tags(&[first], &tags[..15], true).unwrap();
        db.mutate_tags(&[first, first], &[tags[15], tags[15]], true)
            .unwrap();
        assert_eq!(db.file_tags_page(first, None).unwrap().tags.len(), 16);
        db.mutate_tags(&[first], &[tags[0], tags[0]], true).unwrap();
        assert!(
            db.mutate_tags(&[first], &[tags[16]], true)
                .unwrap_err()
                .is::<TagLimitExceeded>()
        );
        // First target could accept the new tag, but the later full target rejects the whole transaction.
        assert!(
            db.mutate_tags(&[second, first], &[tags[16]], true)
                .unwrap_err()
                .is::<TagLimitExceeded>()
        );
        assert!(db.file_tags_page(second, None).unwrap().tags.is_empty());
        assert_eq!(db.file_tags_page(first, None).unwrap().tags.len(), 16);
        db.mutate_tags(&[first], &[tags[0]], false).unwrap();
        db.mutate_tags(&[first], &[tags[16]], true).unwrap();
        assert_eq!(db.file_tags_page(first, None).unwrap().tags.len(), 16);
    }

    #[test]
    fn legacy_over_limit_tags_can_be_removed_but_not_increased() {
        let (_root, db) = fixture();
        db.scan_complete(vec![sample("file", 1)]).unwrap();
        let id = db.list(&query()).unwrap().files[0].id;
        let tags = (0..19)
            .map(|i| db.create_tag(&format!("tag-{i:02}"), None).unwrap())
            .collect::<Vec<_>>();
        legacy_tags(&db, id, &tags[..18]).unwrap();
        db.mutate_tags(&[id], &[tags[0]], true).unwrap();
        assert!(
            db.mutate_tags(&[id], &[tags[18]], true)
                .unwrap_err()
                .is::<TagLimitExceeded>()
        );
        db.mutate_tags(&[id], &tags[..3], false).unwrap();
        db.mutate_tags(&[id], &[tags[18]], true).unwrap();
        assert_eq!(db.file_tags_page(id, None).unwrap().tags.len(), 16);
    }

    #[test]
    fn relink_respects_distinct_union_and_rolls_back_over_limit() {
        let (_root, db) = fixture();
        db.scan_complete(vec![sample("old", 1), sample("new", 2)])
            .unwrap();
        let files = db.list(&query()).unwrap().files;
        let old = files.iter().find(|f| f.path == "old").unwrap().id;
        let new = files.iter().find(|f| f.path == "new").unwrap().id;
        let tags = (0..17)
            .map(|i| db.create_tag(&format!("tag-{i:02}"), None).unwrap())
            .collect::<Vec<_>>();
        db.mutate_tags(&[new], &tags[..16], true).unwrap();
        db.mutate_tags(&[old], &[tags[0], tags[16]], true).unwrap();
        db.scan_complete(vec![sample("new", 2)]).unwrap();
        assert!(db.relink(old, new).unwrap_err().is::<TagLimitExceeded>());
        assert_eq!(db.file_tags_page(old, None).unwrap().tags.len(), 2);
        assert_eq!(db.file_tags_page(new, None).unwrap().tags.len(), 16);
        assert_eq!(
            db.list(&query())
                .unwrap()
                .files
                .iter()
                .find(|f| f.id == old)
                .unwrap()
                .status,
            "missing"
        );
        db.mutate_tags(&[new], &[tags[1]], false).unwrap();
        db.relink(old, new).unwrap();
        assert_eq!(db.file_tags_page(new, None).unwrap().tags.len(), 16);
        assert!(db.file_tags_page(old, None).unwrap().tags.is_empty());
    }

    #[test]
    fn oversized_optional_fields_are_rejected_without_hiding_saved_facts() -> Result<()> {
        let (_root, db) = fixture();
        let tag = db.create_tag("saved", None).unwrap();
        database_operation!(db, 3, connection, {
            sqlx::query("UPDATE tags SET color=? WHERE id=?")
                .bind("x".repeat(1024))
                .bind(tag)
                .execute(&mut *connection)
                .await?;
            sqlx::query("UPDATE roots SET last_error=? WHERE id='main'")
                .bind("x".repeat(2048))
                .execute(&mut *connection)
                .await?;
            Ok::<_, anyhow::Error>(())
        })
        .unwrap();
        assert!(db.tags(None).is_err());
        assert!(db.status().is_err());
        database_operation!(db, 3, connection, {
            let color: String = sqlx::query_scalar("SELECT color FROM tags WHERE id=?")
                .bind(tag)
                .fetch_one(&mut *connection)
                .await?;
            let diagnostic: String =
                sqlx::query_scalar("SELECT last_error FROM roots WHERE id='main'")
                    .fetch_one(&mut *connection)
                    .await?;
            assert_eq!(color.len(), 1024);
            assert_eq!(diagnostic.len(), 2048);
            Ok::<_, anyhow::Error>(())
        })
        .unwrap();
        Ok(())
    }

    #[test]
    fn every_tag_and_file_relation_is_available_across_inverse_pages() {
        let (_root, db) = fixture();
        db.scan_complete(vec![sample("file", 1)]).unwrap();
        let id = db.list(&query()).unwrap().files[0].id;
        let ids = (0..123)
            .map(|i| db.create_tag(&format!("tag-{i:03}"), None).unwrap())
            .collect::<Vec<_>>();
        legacy_tags(&db, id, &ids).unwrap();
        let first = db.tags(None).unwrap();
        assert_eq!(first.tags.len(), 50);
        let second = db.tags(first.next_cursor.as_deref()).unwrap();
        assert_eq!(second.tags.len(), 50);
        let prior = db.tags(second.previous_cursor.as_deref()).unwrap();
        assert_eq!(
            prior.tags.iter().map(|t| t.id).collect::<Vec<_>>(),
            first.tags.iter().map(|t| t.id).collect::<Vec<_>>()
        );
        let third = db.tags(second.next_cursor.as_deref()).unwrap();
        assert_eq!(third.tags.len(), 23);
        assert!(third.next_cursor.is_none());
        let all = first
            .tags
            .into_iter()
            .chain(second.tags)
            .chain(third.tags)
            .map(|t| t.id)
            .collect::<Vec<_>>();
        assert_eq!(all, ids);
        let listed = db.list(&query()).unwrap();
        assert_eq!(listed.files[0].tag_ids.len(), 50);
        assert!(listed.files[0].tag_ids_has_more);
        let mut cursor = None;
        let mut all = Vec::new();
        loop {
            let page = db.file_tags_page(id, cursor.as_deref()).unwrap();
            assert!(page.tags.len() <= 50);
            all.extend(page.tags.into_iter().map(|t| t.id));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(all, ids);
        assert!(
            db.file_tags_page(id, db.tags(None).unwrap().next_cursor.as_deref())
                .unwrap_err()
                .is::<pagination::InvalidCursor>()
        );
    }
    #[test]
    fn all_folder_names_remain_accessible_without_unbounded_materialization() {
        let (_root, db) = fixture();
        db.scan_complete(
            (0..123)
                .map(|i| sample(&format!("_%/folder-{i:03}/file"), i))
                .collect(),
        )
        .unwrap();
        let first = db.folders("_%", None).unwrap();
        assert_eq!(first.folders.len(), 50);
        let second = db.folders("_%", first.next_cursor.as_deref()).unwrap();
        assert_eq!(second.folders.len(), 50);
        assert_eq!(
            db.folders("_%", second.previous_cursor.as_deref())
                .unwrap()
                .folders,
            first.folders
        );
        let third = db.folders("_%", second.next_cursor.as_deref()).unwrap();
        assert_eq!(third.folders.len(), 23);
        assert!(third.next_cursor.is_none());
        let all = first
            .folders
            .into_iter()
            .chain(second.folders)
            .chain(third.folders)
            .collect::<Vec<_>>();
        assert_eq!(
            all,
            (0..123)
                .map(|i| format!("folder-{i:03}"))
                .collect::<Vec<_>>()
        );
        assert!(
            db.folders(
                "another",
                db.folders("_%", None).unwrap().next_cursor.as_deref()
            )
            .unwrap_err()
            .is::<pagination::InvalidCursor>()
        );
    }
    #[test]
    fn capacity_rejects_new_facts_atomically_and_preserves_missing_history() {
        let (_root, mut db) = fixture();
        db.limits.files = 2;
        db.scan_complete(vec![sample("first", 1)]).unwrap();
        db.scan_complete(vec![]).unwrap();
        db.scan_complete(vec![sample("second", 2)]).unwrap();
        let before = serde_json::to_value(db.list(&query()).unwrap()).unwrap();
        let error = db.scan_complete(vec![sample("third", 3)]).unwrap_err();
        assert!(error.is::<Exhausted>());
        assert_eq!(
            serde_json::to_value(db.list(&query()).unwrap()).unwrap(),
            before
        );
        assert_eq!(db.status().unwrap().missing, 1);
        db.limits.tags = 2;
        let a = db.create_tag("a", None).unwrap();
        let b = db.create_tag("b", None).unwrap();
        assert!(db.create_tag("c", None).unwrap_err().is::<Exhausted>());
        assert!(db.update_tag(a, "renamed", None).unwrap());
        assert!(db.delete_tag(b).unwrap());
        db.limits.relations = 1;
        let id = db
            .list(&query())
            .unwrap()
            .files
            .iter()
            .find(|f| f.path == "second")
            .unwrap()
            .id;
        db.mutate_tags(&[id], &[a], true).unwrap();
        db.mutate_tags(&[id], &[a], true).unwrap();
        let c = db.create_tag("c", None).unwrap();
        assert!(
            db.mutate_tags(&[id], &[c], true)
                .unwrap_err()
                .is::<Exhausted>()
        );
        assert_eq!(db.file_tags_page(id, None).unwrap().tags[0].id, a);
        db.mutate_tags(&[id], &[a], false).unwrap();
        assert!(db.file_tags_page(id, None).unwrap().tags.is_empty());
    }
    #[test]
    fn physical_budget_and_disk_floor_refuse_before_insertion() {
        let (_root, mut db) = fixture();
        db.limits.database_bytes = 64 * 1024 * 1024;
        assert!(db.create_tag("denied", None).unwrap_err().is::<Exhausted>());
        assert!(db.tags(None).unwrap().tags.is_empty());
        db.limits = Limits::PRODUCTION;
        db.limits.free_floor = u64::MAX;
        assert!(
            db.scan_complete(vec![sample("denied", 1)])
                .unwrap_err()
                .is::<Exhausted>()
        );
        assert_eq!(db.list(&query()).unwrap().total, 0);
    }
    #[test]
    fn vacuum_backups_keep_exact_current_facts_and_refuse_a_fifth_copy() {
        let (root, db) = fixture();
        db.scan_complete(vec![sample("file", 1)]).unwrap();
        let id = db.list(&query()).unwrap().files[0].id;
        let tag = db.create_tag("kept", None).unwrap();
        db.mutate_tags(&[id], &[tag], true).unwrap();
        let backups = root.path().join("backups");
        std::fs::create_dir(&backups).unwrap();
        std::fs::set_permissions(
            &backups,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
        )
        .unwrap();
        for i in 0..4 {
            let target = backups.join(format!("copy-{i}.db"));
            db.backup(&target).unwrap();
            Database::validate_current(&target, root.path()).unwrap();
            let copied = Database::open(&target, root.path()).unwrap();
            assert_eq!(
                copied.file_tags_page(id, None).unwrap().tags[0].name,
                "kept"
            );
        }
        let paths = std::fs::read_dir(&backups)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect::<Vec<_>>();
        let bytes = paths
            .iter()
            .map(|p| std::fs::read(p).unwrap())
            .collect::<Vec<_>>();
        assert!(
            db.backup(&backups.join("denied.db"))
                .unwrap_err()
                .is::<Exhausted>()
        );
        assert!(!backups.join("denied.db").exists());
        assert_eq!(
            paths
                .iter()
                .map(|p| std::fs::read(p).unwrap())
                .collect::<Vec<_>>(),
            bytes
        );
        assert_eq!(db.file_tags_page(id, None).unwrap().tags[0].name, "kept");
    }
}

#[test]
fn recursive_paths_and_folders_keep_literal_case_sensitive_identity() {
    let t = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    Database::initialize(&t.path().join("db"), t.path()).unwrap();
    let db = Database::open(&t.path().join("db"), t.path()).unwrap();
    let directories = ["Reports", "reports", "数据_%\\A", "数据_%\\a", "数据_XX\\A"];
    let mut samples = Vec::new();
    for (index, directory) in directories.iter().enumerate() {
        for child in ["direct.txt".to_owned(), format!("child-{index}/same.txt")] {
            let path = format!("{directory}/{child}");
            samples.push(Sample {
                parent: path.rsplit_once('/').unwrap().0.into(),
                name: path.rsplit('/').next().unwrap().into(),
                path,
                dev: 1,
                ino: samples.len() as i64 + 1,
                size: 1,
                mtime_ns: 1,
                ctime_ns: 1,
            });
        }
    }
    db.scan_complete(samples).unwrap();
    for (index, directory) in directories.iter().enumerate() {
        let query = FileQuery {
            search: String::new(),
            directory: (*directory).into(),
            scope: "recursive".into(),
            status: "present".into(),
            all_tags: vec![],
            any_tags: vec![],
            exclude_tags: vec![],
            page: 1,
            page_size: 50,
        };
        let mut expected = vec![
            format!("{directory}/direct.txt"),
            format!("{directory}/child-{index}/same.txt"),
        ];
        expected.sort();
        let page = db.list(&query).unwrap();
        let mut paths: Vec<_> = page.files.into_iter().map(|file| file.path).collect();
        paths.sort();
        assert_eq!(page.total, 2);
        assert_eq!(paths, expected);
        let mut matched: Vec<_> = db.matching_tag_paths(&query).unwrap().into_keys().collect();
        matched.sort();
        assert_eq!(matched, expected);
        assert_eq!(
            db.folders(directory, None).unwrap().folders,
            vec![format!("child-{index}")]
        );
    }
}

#[test]
fn case_sensitive_folder_cursors_keep_forward_and_reverse_scope() {
    let t = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    Database::initialize(&t.path().join("db"), t.path()).unwrap();
    let db = Database::open(&t.path().join("db"), t.path()).unwrap();
    let samples = (0..52)
        .map(|index| {
            let parent = if index == 51 {
                "reports/foreign".into()
            } else {
                format!("Reports/child-{index:02}")
            };
            Sample {
                path: format!("{parent}/same.txt"),
                parent,
                name: "same.txt".into(),
                dev: 1,
                ino: index,
                size: 1,
                mtime_ns: 1,
                ctime_ns: 1,
            }
        })
        .collect();
    db.scan_complete(samples).unwrap();
    let first = db.folders("Reports", None).unwrap();
    assert_eq!(
        first.folders,
        (0..50).map(|i| format!("child-{i:02}")).collect::<Vec<_>>()
    );
    let last = db.folders("Reports", first.next_cursor.as_deref()).unwrap();
    assert_eq!(last.folders, vec!["child-50"]);
    assert!(last.next_cursor.is_none());
    let previous = db
        .folders("Reports", last.previous_cursor.as_deref())
        .unwrap();
    assert_eq!(previous.folders, first.folders);
}

#[test]
fn tag_ids_survive_empty_table_and_reopen() {
    let root = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let path = root.path().join("tags.db");
    Database::initialize(&path, root.path()).unwrap();
    let db = Database::open(&path, root.path()).unwrap();
    let first = db.create_tag("first", None).unwrap();
    let second = db.create_tag("second", None).unwrap();
    assert!(db.delete_tag(second).unwrap());
    let third = db.create_tag("third", None).unwrap();
    assert!(third > second);
    assert!(db.delete_tag(first).unwrap());
    assert!(db.delete_tag(third).unwrap());
    drop(db);
    let reopened = Database::open(&path, root.path()).unwrap();
    assert!(reopened.create_tag("after reopen", None).unwrap() > third);
}

#[test]
fn old_reusable_tag_schema_is_rejected_without_mutation() {
    let root = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let path = root.path().join("tags.db");
    std::fs::File::create(&path).unwrap();
    xcss::sqlite::block_on_sqlite_connection(async {
        let mut connection = connect(&path).await.unwrap();
        sqlx::raw_sql(include_str!("fixtures/tagging-rowid.sql"))
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx::query("INSERT INTO roots(id,path) VALUES('main',?)")
            .bind(root.path().to_str().unwrap())
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx::query("INSERT INTO tags(name) VALUES('retained')")
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
    });
    let before = std::fs::read(&path).unwrap();
    let error = Database::validate_current(&path, root.path()).unwrap_err();
    assert!(error.to_string().contains("exact current structure"));
    assert!(Database::open(&path, root.path()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn prepared_tag_open_rejects_replaced_source_generation() {
    let root = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let path = root.path().join("tags.db");
    Database::initialize(&path, root.path()).unwrap();
    let prepared = Database::prepare_in(&path, root.path(), root.path()).unwrap();
    let original = std::fs::read(&path).unwrap();
    let replacement = root.path().join("replacement.db");
    std::fs::write(&replacement, &original).unwrap();
    std::fs::rename(&replacement, &path).unwrap();
    let error = prepared.open().err().expect("replaced source was opened");
    assert!(
        error
            .downcast_ref::<xcss::sqlite::SnapshotError>()
            .is_some()
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(!path.with_file_name("tags.db-wal").exists());
}
