# 安装并管理第一个文件

使用 Linux AMD64 GNU 主机，内核提供 `openat2`，系统 glibc 与下载程序匹配。目标是经 HTTPS 登录，在测试目录上传并读回一个文件。
下面的安装示例由有 sudo 权限的管理员执行；服务使用独立 `xczs` 用户。

## 1. 准备程序

[下载页](https://github.com/isarmg/xczs/releases)的便捷资产是单个程序与同名 `.sha256`：

```sh
sha256sum --check xczs-1.0.3-x86_64-unknown-linux-gnu.sha256
chmod 0755 xczs-1.0.3-x86_64-unknown-linux-gnu
XCZS_BIN="$PWD/xczs-1.0.3-x86_64-unknown-linux-gnu"
```

如从源码构建，在干净仓库根目录准备 Rust 1.99.0、Node.js 26.7.0 和 C 工具链：

```sh
rustup target add --toolchain 1.99.0 x86_64-unknown-linux-gnu
npm ci --ignore-scripts --no-audit --no-fund
npm run build:server:release -- --no-install
XCZS_BIN="$PWD/target/x86_64-unknown-linux-gnu/release/xczs"
```

输出 `target/x86_64-unknown-linux-gnu/release/xczs`。下面用 `XCZS_BIN` 指向所选程序，安装时复制到 `/opt/xczs/bin/xczs`。
完整签名运行包还包含部署样例与清单，使用其[包内手册](runtime-package.md)；它与单文件下载是不同交付方式。

## 2. 准备账号和目录

先按主机规范创建不可登录的 `xczs` 用户和同名组。以下目录用于全新测试部署：

```sh
sudo install -d -o root -g root -m 0755 /opt/xczs/bin
sudo install -d -o root -g xczs -m 0750 /etc/xczs
sudo install -d -o xczs -g xczs -m 0700 /var/lib/xczs
sudo install -d -o xczs -g xczs -m 0750 /srv/xczs
sudo install -o root -g root -m 0755 "$XCZS_BIN" /opt/xczs/bin/xczs
/opt/xczs/bin/xczs hash-password
```

最后一条交互读取密码并输出 Argon2id PHC。将完整哈希写入 `/etc/xczs/xczs.json`：

```json
{
  "serve_path": "/srv/xczs",
  "data_dir": "/var/lib/xczs",
  "bind": ["127.0.0.1"],
  "port": 5000,
  "auth": ["admin:$argon2id$REPLACE_WITH_THE_GENERATED_HASH"]
}
```

配置设为 `root:xczs 0640`，使用无扩展 access ACL 的单硬链接普通文件。共享根与私有状态目录分开，配置放在共享根外。
共享根由 xczs 独占写入；人工文件维护在停服后进行，以便上传、标签与文件身份保持一致。

## 3. 初始化并检查

```sh
sudo -u xczs /opt/xczs/bin/xczs init --config /etc/xczs/xczs.json
sudo -u xczs /opt/xczs/bin/xczs config validate --config /etc/xczs/xczs.json
```

`init` 为全新私有状态目录创建数据库、标签和管理员状态；已有实例只执行校验。
正常结果是配置与完整当前状态均有效。状态库绑定共享根身份，已有共享树和状态应作为一组使用。

## 4. 配置 HTTPS 并运行

将 HTTPS 代理转发到 `127.0.0.1:5000`，覆盖规范 Host 并限制其他客户端直连后端。源码和完整运行包的 `deploy/` 提供 nginx 与 systemd 样例，安装方法见[运维](operations.md)。

```sh
sudo -u xczs /opt/xczs/bin/xczs run --config /etc/xczs/xczs.json
```

在另一终端运行：

```sh
curl --fail http://127.0.0.1:5000/readyz
```

正常应返回已就绪。浏览器打开实际 HTTPS 域名，登录后上传测试文件并下载核对。完成后按[文件与标签指南](usage.md)使用；前台 Ctrl+C 可停止服务，再配置 systemd 常驻。
