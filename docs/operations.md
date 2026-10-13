# 部署与日常运维

首次安装、升级和目录权限见[运行包安装手册](runtime-package.md)，参数见[CLI 与配置](cli.md)。统一通过包根 `install.sh setup CONFIG_JSON [USERNAME]` 安装，管理员初始密码由标准输入提供。安装完成后自行执行 `systemctl enable --now xczs.service`。升级执行 `install.sh upgrade`，只恢复升级前处于运行状态的服务。

## 配置和权限

普通服务账号为 `isarmg-xczs`。程序在 `/opt/isarmg/xczs/bin/xczs`，默认配置在 `/etc/isarmg/xczs/config.json`，内部状态在 `/var/lib/isarmg/xczs`。配置文件属于服务账号，权限 `0600`，父目录 `0700`。运行目录由 systemd 管理。

`serve_path` 必须显式填写绝对路径，与 `data_dir` 互不包含；同一共享根仅由一个实例管理。安装器根据实际配置生成 systemd 写入白名单，不递归修改共享文件所有者。修改根路径后应重新审阅 unit 白名单。

生产通过自行管理的 HTTPS 网关访问。程序不提供代理软件配置。`trusted_proxies` 默认空，只有明确列出的可信连接来源能提供有效转发地址；不要把其他本机进程默认视为可信代理。开发模式只能显式用于回环 HTTP。

## 检查与维护

```sh
sudo -u isarmg-xczs /opt/isarmg/xczs/bin/xczs config validate
sudo systemctl stop xczs.service
sudo -u isarmg-xczs /opt/isarmg/xczs/bin/xczs check
sudo systemctl start xczs.service
sudo -u isarmg-xczs /opt/isarmg/xczs/bin/xczs status --json
sudo journalctl -u xczs --since today --no-pager
```

`config validate` 只验证配置；`check` 离线检查现有状态、账户、数据库和共享根；`status` 查询在线 readiness。重置已有管理员密码需先停服，使用 `admin reset-password --username admin` 并从标准输入传入新密码。普通启动不会初始化或重置账户。

文件日志固定为 `data_dir/logs/xczs.jsonl`，采用 UTC 结构化记录，每份最多 8 MiB，保留四个归档。`log_level` 默认 `info`。对外提供诊断信息前移除账号、Cookie、CSRF、凭据和文件内容。

停服通过 systemd。程序停止准入后给普通工作和已登记提交 30 秒宽限，再给排空 10 秒，日志刷新最多 5 秒；unit 的 120 秒超时为外层边界。人工写入共享根应放在停服窗口。

## 数据一致性与故障

状态目录包含 `state.sqlite3`、`tags.db`、管理员记录、日志和 sidecar；共享根包含用户文件以及受管理的上传、删除和隔离对象。备份应保留两者的一致时间点及根绑定。状态库绑定根设备号和 inode，空状态目录不能代替已有状态。

服务状态页的标签备份保存在 `data_dir/tag-backups/`，不构成整站备份。`.xczs-upload-stages`、`.xczs-upload-delete-<uuid>.trash` 和 `.xczs-quarantine-<uuid>.hold` 为内部对象，不应在线手动修改。隔离对象应先停服、核对来源和内容再处理。

监控 5xx/429/507、空间、inode、挂载与登录限流。结构校验失败应保留数据库和 sidecar，核对程序与共享根身份。疑似入侵先限制入口、保全摘要与时间线，再处理凭据。具体故障见[排查指南](troubleshooting.md)，发行验证见[发行指南](releasing.md)。
