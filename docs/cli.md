# 当前服务命令

所有命令使用同一份当前 JSON 配置：`xczs --config /absolute/config.json --data-dir /absolute/data ...`。配置来源优先级为命令行、明确映射的环境变量、文件、默认值。未知字段、重复字段、错误类型和坏的低优先级配置都会被拒绝；诊断不会打印秘密值。`config validate --json` 中 `sources` 是字段的来源，`schema_identity` 是校验过的当前结构身份，`state_paths` 列出全部私有持久状态的绝对路径。

```sh
xczs --help
xczs --version
xczs init --config /absolute/config.json --data-dir /absolute/data
xczs config validate --config /absolute/config.json --data-dir /absolute/data --json
xczs run --config /absolute/config.json --data-dir /absolute/data
xczs status --config /absolute/config.json --data-dir /absolute/data --json
```

`init` 是创建当前状态库、标签库及已配置管理员私有凭据存储的唯一部署入口，只接受尚未初始化的私有空目录。管理员先在 `auth` 中声明，密码通过 `hash-password` 生成符合当前策略的 Argon2id PHC；`init` 不提供交互创建账号或 `--username` 参数。配置和管理员凭据必须先有效；已有数据不会被覆盖。数据目录应属于服务账号、权限为 `0700`，私有文件权限为 `0600`。Xczs 发行包沿用其编译绑定的发行身份校验，运行命令是 `run`。

`run` 只接受完整的当前数据；缺失数据库、管理员、结构漂移或身份不符均失败，不隐式初始化或重置账号。运行及写入维护命令使用同一个数据目录维护锁；整个运行期间持有实例锁。`.state-maintenance-pending.json` 存在时拒绝运行、初始化或写入维护；只读检查仍可运行。

`config validate` 用私有 SQLite 快照检验状态库和标签库，并只读验证静态管理员文件；原库、WAL、SHM 和业务文件保持不变。`status` 查询当前监听地址的 `/readyz`，核对服务身份和真实业务就绪；端口占用、其他服务、连接失败或未就绪均返回非零退出码。`--json` 输出单个机器记录；失败返回稳定 `code/message/details` 错误记录。帮助和版本查询不要求初始化。

服务运行时，活动写事务可能使 `config validate --json` 返回 `snapshot.busy`；快照采集前后发生正常数据库提交、WAL 更新或 journal 创建、移除时，可能返回 `snapshot.source_changed`。这两个错误的 `retryable` 为 `true`，本次检查仍以非零退出码结束；应等待写入停止后重新执行原命令，不能据此重置数据。检查仍会拒绝文件身份、权限、链接或结构不符的状态；不能把其他错误一律当成并发写入重试。

共享配置、命令、快照与日志均固定到同一 xcss Git 完整提交和精确版本；Web 包使用封存制品的真实 SHA-512 完整性。每次正式发行从这些精确输入独立构建并验证最终制品，源码检查和 Linux 发行物验收分别记录。

Xczs 使用 `serve_path`、`data_dir`、`auth` 等 snake_case JSON 字段；命令行共享根为 `--serve-path /absolute/shared-root`。状态目录与共享根均需完整备份；人工标签、管理员文件和上传/操作状态不可由文件扫描恢复。

`init` 同时创建 `data_dir/logs` 私有目录；`run` 验证该目录后写入共享 JSON 日志。默认单文件上限 8 MiB、保留 4 个归档，总上限 40 MiB。配置和状态命令不打开运行日志文件。

## 环境变量

仅下列 19 个变量映射到运行时配置；未声明的变量不会自动映射。`XCZS_BIND` 和 `XCZS_AUTH` 使用完整 JSON 数组，布尔值使用 `true` / `false`，整数字段使用无符号十进制数。命令行显式参数优先于这些变量，其次是 JSON 文件与默认值；每层的无效值都会使命令失败，不能靠更高层覆盖隐藏错误。

| 环境变量 | 配置字段 | 作用 |
|---|---|---|
| `XCZS_SERVE_PATH` | `serve_path` | 指定共享文件根目录，使用绝对路径 |
| `XCZS_DATA_DIR` | `data_dir` | 指定私有状态目录，使用绝对路径 |
| `XCZS_DEVELOPMENT` | `development` | 显式选择仅限回环地址的开发 HTTP 模式 |
| `XCZS_BIND` | `bind` | 指定监听 IP 地址数组，例如 `["127.0.0.1"]` |
| `XCZS_PORT` | `port` | 指定监听端口 |
| `XCZS_AUTH` | `auth` | 指定管理员名称和 Argon2id PHC 数组，不使用明文密码 |
| `XCZS_LOG_FORMAT` | `log_format` | 指定日志记录格式 |
| `XCZS_LOG_FILE` | `log_file` | 指定日志文件路径 |
| `XCZS_MAX_UPLOAD_SIZE` | `max_upload_size` | 限制单个上传文件的字节数 |
| `XCZS_UPLOAD_IDLE_TIMEOUT` | `upload_idle_timeout` | 限制上传正文连续无数据的秒数 |
| `XCZS_UPLOAD_TOTAL_TIMEOUT` | `upload_total_timeout` | 限制单次上传请求的总秒数 |
| `XCZS_MAX_CONCURRENT_UPLOADS` | `max_concurrent_uploads` | 限制同时占用上传槽的请求数量 |
| `XCZS_MIN_FREE_SPACE` | `min_free_space` | 指定共享根需要保留的可用字节数 |
| `XCZS_MAX_CONNECTIONS` | `max_connections` | 限制并发连接数量 |
| `XCZS_MAX_SEARCH_ENTRIES` | `max_search_entries` | 限制一次搜索遍历的条目数 |
| `XCZS_MAX_CONCURRENT_SEARCHES` | `max_concurrent_searches` | 限制同时执行的搜索数量 |
| `XCZS_REQUEST_TIMEOUT` | `request_timeout` | 限制普通请求总秒数 |
| `XCZS_TAG_SCAN_INTERVAL_SECONDS` | `tag_scan_interval_seconds` | 指定标签扫描间隔秒数 |
| `XCZS_MAX_TAG_SCAN_ENTRIES` | `max_tag_scan_entries` | 限制标签扫描遍历的条目数 |

部署时优先使用受保护的 JSON 文件保存管理员 PHC；不要把凭据或完整私有配置复制到命令历史和公开日志。`config validate --json` 只读核对有效配置、字段来源和当前状态。修改运行配置后，需要按[运维说明](operations.md)重启服务。

访问日志时间统一输出 UTC RFC3339（`Z`）。`$time_iso8601` 与 `$time_utc` 使用同一 UTC 时刻。文件修改时间与完整扫描时间也显示 UTC。
