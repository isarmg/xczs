# 运行入口与配置

server 只支持运行、帮助和版本输出：

```sh
ROOT/bin/xczs --root ROOT
ROOT/bin/xczs --help
ROOT/bin/xczs --version
```

默认 ROOT `/opt/isarmg/xczs`。配置固定 `ROOT/config/config.json`，必须是服务账号拥有的 0600 普通单硬链接文件，父目录 0700。配置缺失、权限错误、未知字段或旧配置都失败关闭。环境变量使用产品前缀 `XCZS_`，优先级为环境、JSON、默认值，server CLI 不提供业务配置覆盖。

维护由 xssi 的 `maintenance.sh --root ROOT check`、`admin-reset --username NAME` 执行；签名 `libexec/setup` 提供配置校验、显式初始化、只读检查和密码重置。密码来自标准输入，不进入 server argv、配置或日志。

- `serve_path`：业务共享根，已存在的绝对目录。
- `data_dir`：默认 ROOT/state，数据库 state.sqlite3、tags.db 和公共锁所在地，可明确外置实例业务数据。
- `bind`：默认 127.0.0.1:5000，单个 SocketAddr。
- `development`：默认 false，仅允许回环 HTTP；正式源码绑定程序拒绝开发资源目录覆盖。
- `max_upload_size`：默认 100 GiB；上传并发默认 4。
- `max_connections`：默认 256。
- `max_concurrent_searches`：默认 2，单次搜索默认最多 10000 个条目。
- `upload_idle_timeout`、`upload_total_timeout`、`request_timeout`：上传和请求资源边界。
- `log_level`、`log_format`：日志写入 ROOT/logs，不支持来源 IP 模板变量。
- `tag_scan_interval_seconds`、`max_tag_scan_entries`：标签扫描容量。

管理员与会话统一存入 state.sqlite3，不支持管理员配置值或 JSON 账户文件。不支持可信代理、转发 IP、旧环境变量别名或旧 CLI。[配置样例](../config/config.json.example)。
