# xczs

## 项目简要介绍

面向个人或受控局域网的轻量 Web 文件管理器。一个 Rust 进程同时提供管理页面和文件服务。

## 项目功能

- 文件浏览、搜索、上传续传、下载、移动、重命名和删除
- 文件标签管理、组合筛选和索引扫描
- 管理员认证、运行状态与结构化操作反馈

## 适用平台

仅支持 Linux AMD64 GNU（`x86_64-unknown-linux-gnu`），内核必须提供 `openat2`，并满足制品的 glibc 要求。生产环境使用 HTTPS 反向代理；共享根应由 xczs 独占写入。

## 如何快速部署

从 [下载页](https://github.com/isarmg/xczs/releases) 取得 Linux 二进制和同名 `.sha256` 文件。公开便捷发行物是单个程序，不含安装器或配置模板。

```sh
sha256sum --check xczs-1.0.3-x86_64-unknown-linux-gnu.sha256
chmod 0755 xczs-1.0.3-x86_64-unknown-linux-gnu
./xczs-1.0.3-x86_64-unknown-linux-gnu hash-password
```

准备专用服务账号 `xczs`、共享目录 `/srv/xczs` 和该账号拥有的 `0700` 状态目录 `/var/lib/xczs`。创建 `/etc/xczs/xczs.json`，由 `root:xczs` 拥有并设为 `0640`，填入以下内容并替换完整密码哈希：

```json
{
  "serve_path": "/srv/xczs",
  "data_dir": "/var/lib/xczs",
  "bind": ["127.0.0.1"],
  "port": 5000,
  "auth": ["admin:$argon2id$REPLACE_WITH_THE_GENERATED_HASH"]
}
```

确保账号可管理共享根，配置文件位于共享根外，并能读取二进制后执行：

```sh
sudo -u xczs ./xczs-1.0.3-x86_64-unknown-linux-gnu init --config /etc/xczs/xczs.json
sudo -u xczs ./xczs-1.0.3-x86_64-unknown-linux-gnu config validate --config /etc/xczs/xczs.json
sudo -u xczs ./xczs-1.0.3-x86_64-unknown-linux-gnu run --config /etc/xczs/xczs.json
```

仅全新状态执行 `init`。将 HTTPS 网关转发至 `127.0.0.1:5000`，确认能登录和操作文件后再配置 systemd 常驻。

## 如何编译部署

在 Linux AMD64 的干净源码目录准备 Rust 1.99.0、Node.js 26.7.0 和 C 编译工具：

```sh
rustup target add --toolchain 1.99.0 x86_64-unknown-linux-gnu
npm ci --ignore-scripts --no-audit --no-fund
npm run build:server:release -- --no-install
```

程序输出到 `target/x86_64-unknown-linux-gnu/release/xczs`，前端已嵌入。使用该程序替换上面的下载文件，按同一流程配置和启动；源码 `config/` 与 `deploy/` 提供完整配置、systemd 和 nginx 示例。

[详细文档](docs/README.md)
