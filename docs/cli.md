# 当前服务命令

所有命令共享当前 JSON 配置；优先级为显式 CLI > `XCZS_*` 环境变量 > JSON > 默认值。每层独立严格校验，不用高优先级值隐藏坏配置。未知字段、重复字段和错误类型被拒绝，诊断不输出秘密。

```sh
xczs --help
xczs init --config /absolute/config.json --username admin
xczs config validate --config /absolute/config.json --json
xczs check --config /absolute/config.json --json
xczs run --config /absolute/config.json
xczs status --config /absolute/config.json --json
xczs admin reset-password --config /absolute/config.json --username admin
```

`init` 和 `admin reset-password` 从 stdin 读取密码到 EOF，允许一个末尾换行；推荐 `systemd-ask-password` 或秘密管理器管道。没有配置哈希 bootstrap 或 hash-password 命令。init 只接受新状态，run 不会创建或重置账户。网页内的账号改名/改密仍由统一管理员服务管理，文件 ACL 使用不变的稳定 ID。

`config validate` 仅验证值、来源和配置文件权限，不要求 state/数据库/账号存在，也不写状态。`check` 只读检查当前目录、SQLite 快照、schema 和管理员文件。`status` 查询活动实例，核对服务身份与真实 readiness。维护命令和运行进程互斥；存在维护标记时拒绝运行与写入维护。

## 配置

- 全局 `--config` 默认 `/etc/isarmg/xczs/config.json`。隐式默认文件不存在可用 CLI/环境配置；显式文件缺失、权限或格式错误会失败。
- 配置由服务账号拥有，单硬链接文件 `0600`、父目录 `0700`。
- `data_dir` 默认 `/var/lib/isarmg/xczs`，是唯一内部状态根。数据库、管理员和日志均放在其中。
- `serve_path` 必填绝对路径，与状态根互不包含，不默认工作目录。
- `bind` 是单个 SocketAddr 字符串，默认 `127.0.0.1:5000`；IPv6 示例 `[::1]:5000`。
- `development` 默认 false，只能配合 loopback HTTP；`--development=false` 可显式覆盖。
- `trusted_proxies` 默认 []，为 IP/CIDR 字符串数组。只有 socket peer 受信时解析 X-Forwarded-For，从右向左选第一个不受信地址；无效受信链被拒绝。日志、认证限流、审计共享一次解析结果。不用 Forwarded/X-Forwarded-Proto 决定认证来源或协议。
- `log_level` 默认 info，可选 off/error/warn/info/debug/trace。共享 UTC JSON 日志固定在 `data_dir/logs/xczs.jsonl`，每文件 8 MiB、保留 4 个归档。配置验证和离线检查不打开运行日志。

当前 CLI 不接受独立 port、IP 数组、auth bootstrap 或任意 log_file 路径，也不提供旧格式别名。

## 环境变量

| 变量 | JSON 字段 | 格式 |
|---|---|---|
| XCZS_SERVE_PATH | serve_path | 绝对路径 |
| XCZS_DATA_DIR | data_dir | 绝对路径 |
| XCZS_BIND | bind | SocketAddr |
| XCZS_DEVELOPMENT | development | true/false |
| XCZS_TRUSTED_PROXIES | trusted_proxies | JSON IP/CIDR 数组 |
| XCZS_LOG_LEVEL | log_level | 小写级别 |
| XCZS_LOG_FORMAT | log_format | 访问日志格式 |
| XCZS_MAX_UPLOAD_SIZE | max_upload_size | 字节整数 |
| XCZS_UPLOAD_IDLE_TIMEOUT | upload_idle_timeout | 秒整数 |
| XCZS_UPLOAD_TOTAL_TIMEOUT | upload_total_timeout | 秒整数 |
| XCZS_MAX_CONCURRENT_UPLOADS | max_concurrent_uploads | 整数 |
| XCZS_MIN_FREE_SPACE | min_free_space | 字节整数 |
| XCZS_MAX_CONNECTIONS | max_connections | 整数 |
| XCZS_MAX_SEARCH_ENTRIES | max_search_entries | 整数 |
| XCZS_MAX_CONCURRENT_SEARCHES | max_concurrent_searches | 整数 |
| XCZS_REQUEST_TIMEOUT | request_timeout | 秒整数 |
| XCZS_TAG_SCAN_INTERVAL_SECONDS | tag_scan_interval_seconds | 秒整数 |
| XCZS_MAX_TAG_SCAN_ENTRIES | max_tag_scan_entries | 整数 |

源码开发资源目录仅使用 `XCZS_DEV_WEB_DIR`，必须是 unbound 开发构建并显式开启 development；它不是发行包要求。状态检查遇到 snapshot.busy/source_changed 时可等待写入暂停后重试，不要重置数据。
