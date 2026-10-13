# xczs 完整运行包

本包包含 `bin/xczs`、`config/config.json.example`、`systemd/xczs.service`、根目录 `install.sh`、内部安装实现 `libexec/`、本手册、许可证、SBOM、构建记录和校验文件。Web 已内嵌，不依赖发行目录布局，也不附带反向代理软件或配置。

## 1. 验证制品

先验证发布渠道提供的签名与归档摘要，再在解压目录执行：

```sh
sha256sum --check SHA256SUMS
./bin/xczs --version
./bin/xczs web-assets | cmp - WEB-ASSETS.json
```

## 2. 准备当前 JSON

复制 `config/config.json.example` 到自己选择的配置文件，核对：
- `serve_path`：必填、已明确选择的绝对共享路径，样例为 `/srv/xczs`
- `data_dir`：唯一内部状态根，默认 `/var/lib/isarmg/xczs`
- `bind`：单个 SocketAddr，默认 `127.0.0.1:5000`
- `development`：默认 `false`；仅本机 HTTP 开发时显式开启
- `log_level`：默认 `info`；日志固定在 `data_dir/logs`
- `trusted_proxies`：默认空列表；只有明确受信的 socket peer 才允许提供 X-Forwarded-For

生产 HTTPS 入口应覆写规范 Host 和转发信息，并通过防火墙或网络隔离防止不可信来源直连后端。配置没有密码、哈希或管理员 bootstrap 字段。

## 3. 首次安装

```sh
systemd-ask-password 'Initial administrator password:' | sudo ./install.sh setup /absolute/config.json admin
```

入口为 `install.sh setup CONFIG_JSON [USERNAME]`，用户名默认 `admin`，密码从 stdin 读取。安装器创建普通系统账号 `isarmg-xczs`，安装程序到 `/opt/isarmg/xczs/bin/xczs`，配置到 `/etc/isarmg/xczs/config.json`。配置文件为服务账号拥有的 `0600` 单硬链接文件，父目录 `0700`。

默认共享根不存在时创建 `/srv/xczs`；已有共享数据不递归 chown。自定义共享根必须预先存在并允许服务账号访问。自定义状态目录须预先由服务账号拥有且为 `0700`。安装器依据实际 JSON 生成 unit 可写白名单，内部日志不能另选任意路径。

`setup` 显式执行 `init` 和 `check`，不自动启动服务。确认 HTTPS 入口和目录权限后：

```sh
sudo systemctl enable --now xczs.service
sudo -u isarmg-xczs /opt/isarmg/xczs/bin/xczs status --json
```

systemd 管理 `/run/isarmg/xczs`，应用管理状态根下的数据库、管理员文件和轮转日志。诊断也可查看 `journalctl -u xczs.service`。

## 4. 安装更新

在新运行包根目录执行 `sudo ./install.sh upgrade`。该命令使用现有当前格式配置，不导入或转换其他格式，不重新初始化账号或数据库。先验证候选程序与 unit，停止原服务后执行离线 `check`；通过后替换程序，仅原本活动的服务会重启。校验或重启失败时保留数据并恢复原二进制，报告失败。

安装前备份完整状态根与共享数据。程序不自动迁移未知数据结构；`check` 不通过时应调查失败原因。

## 5. 离线管理与诊断

```sh
sudo -u isarmg-xczs /opt/isarmg/xczs/bin/xczs config validate --json
sudo systemctl stop xczs.service
sudo -u isarmg-xczs /opt/isarmg/xczs/bin/xczs check --json
systemd-ask-password 'New administrator password:' | sudo -u isarmg-xczs /opt/isarmg/xczs/bin/xczs admin reset-password --username admin
sudo systemctl start xczs.service
```

`config validate` 仅验证当前配置，不要求数据库或账号存在，也不创建状态。`check` 只读验证目录、结构、账号和业务就绪前提；`status` 查询活动进程的 `/readyz`。密码重置持有离线维护锁，保留稳定账号 ID 并递增 session version；运行中的服务会阻止维护。

完整文档见[项目文档](https://github.com/isarmg/xczs/blob/main/docs/README.md)，请选择与制品源码 SHA 对应的版本。
