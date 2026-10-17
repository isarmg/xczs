# xczs

自托管文件管理服务，提供文件操作、可恢复上传、下载、标签及管理员工作区。仅支持 Linux AMD64 GNU。

server 只负责运行，入口为 `bin/xczs --root ROOT`。配置默认 `ROOT/config/config.json`，数据库默认 `ROOT/state/state.sqlite3`，标签库 `ROOT/state/tags.db`，日志 `ROOT/logs`，临时文件 `ROOT/run`。共享文件与实例业务状态可明确外置，systemd 文件使用系统目录。服务不识别代理或访客 IP，不解析转发地址头。

## 安装和维护

安装与维护来自独立 [xssi](https://github.com/isarmg/xssi) 仓库，以 `config/xssi.lock` 的完整提交固定。SHA-256 文件清单覆盖 server、辅助工具、资源和 systemd 模板。安装工具核验包内清单及每个文件的 SHA-256、大小和权限；升级使用专用工具。

```sh
systemd-ask-password 'Initial administrator password:' | sudo ./install.sh \
  --package /absolute/verified-package --root /opt/isarmg/xczs \
  --config /absolute/private-config.json --username admin
sudo systemctl enable --now xczs.service
curl --fail http://127.0.0.1:5000/readyz
```

首次安装由脚本检查目录、权限和账号，独立辅助程序创建当前数据库和管理员，生成 systemd 配置后保持停服。共享目录可包含已有文件，不递归修改权限或删除内容。

管理员凭据、会话及管理员审计与业务状态统一存入 `state.sqlite3` 的公共表；密码为 Argon2id 哈希，文件所有权仍绑定稳定 `administrator_id`。不读写管理员 JSON，不自动迁移旧结构。

升级执行 `ROOT/upgrade.sh --root ROOT --version VERSION --xssc /absolute/xssc`：自动下载、SHA-256 校验、停服、检查与备份、执行明确支持的数据步骤、替换、启动并核对真实就绪。未知旧结构和旧发行布局直接拒绝。卸载由用户自行处理。

## 构建

生产 Web 构建使用固定 Rust 1.99.0 和独立 Cargo 锁图；正式打包额外需要 Python 3.11+。浏览器前端使用 Rust / xcss-web CSR，后端保留 Axum 并兼容既有 API，按 WASM 加载、缓存与传输需要适配优化。先执行 `bash scripts/install-xcss-web-tools.sh` 准备固定 WASM target、Rustfmt/Clippy 和 wasm-bindgen 0.2.129；开发与验收见 [xcss-web 浏览器维护](docs/xcss-web-browser.md)。正式打包要求干净源码、同版本 annotated tag、固定已发布 xssi 提交。

生产前端构建使用仓库内 Rust 工具，无需安装 npm 依赖。准备 Rust 1.99.0、wasm32-unknown-unknown target 和 wasm-bindgen-cli 0.2.129 后，在产品根目录执行：

```sh
cargo +1.99.0 run --quiet --locked --manifest-path tools/web-build/Cargo.toml -- build --product-root .
```

开发监听、Axum 集成和测试边界见 [Rust 原生 Web 构建与测试](docs/rust-native-web.md)。Node/Playwright/Vite 仅用于浏览器验收与历史 TypeScript/React 对照测试。

```sh
output="$(mktemp -d /tmp/xczs-release.XXXXXXXX)"
bash scripts/package-release.sh --output-dir "$output"
```

输出 `xczs-VERSION-linux-x86_64.tar.gz` 和 `SHA256SUMS`；不再发布仅有裸二进制的安装资产。[完整文档](docs/README.md)。
