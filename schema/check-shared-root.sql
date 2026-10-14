SELECT (SELECT COUNT(*) FROM store_meta) = 2
 AND (SELECT value FROM store_meta WHERE key='root-device-be') = ?1
 AND (SELECT value FROM store_meta WHERE key='root-inode-be') = ?2
