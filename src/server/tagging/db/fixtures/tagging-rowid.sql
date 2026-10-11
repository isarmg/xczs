            CREATE TABLE IF NOT EXISTS roots (
              id TEXT PRIMARY KEY, path TEXT NOT NULL, available INTEGER NOT NULL DEFAULT 0,
              scan_generation INTEGER NOT NULL DEFAULT 0, last_scan_at INTEGER, last_error TEXT
            );
            CREATE TABLE IF NOT EXISTS files (
              id INTEGER PRIMARY KEY, root_id TEXT NOT NULL REFERENCES roots(id),
              path TEXT NOT NULL, parent TEXT NOT NULL, name TEXT NOT NULL,
              dev INTEGER NOT NULL, ino INTEGER NOT NULL, size INTEGER NOT NULL,
              mtime_ns INTEGER NOT NULL, ctime_ns INTEGER NOT NULL,
              status TEXT NOT NULL CHECK(status IN ('present','missing','suspect','relinked')),
              last_seen_generation INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS files_path_status ON files(root_id,path,status);
            CREATE INDEX IF NOT EXISTS files_parent_status ON files(parent,status);
            CREATE TABLE IF NOT EXISTS tags (
              id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE,
              color TEXT
            );
            CREATE TABLE IF NOT EXISTS file_tags (
              file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
              tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
              PRIMARY KEY(file_id,tag_id)
            );
            CREATE INDEX IF NOT EXISTS file_tags_tag_file ON file_tags(tag_id,file_id);
