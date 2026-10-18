# 环境与首次运行

生产构建使用 Rust 1.99.0、wasm32-unknown-unknown target 和 wasm-bindgen-cli 0.2.129，无需 npm 安装；在产品根目录执行 `cargo +1.99.0 run --quiet --locked --manifest-path tools/web-build/Cargo.toml -- server --product-root . --config xcss-web-build.json --mode development`。浏览器及工具验证另需 `.node-version` 固定的 Node 26.7.0 与 `npm ci --ignore-scripts --no-audit --no-fund` 安装的锁定开发依赖。命令详见 [Rust 原生 Web 构建](../rust-native-web.md)，正式发行见[发布说明](../releasing.md)。

server 入口只有 `--root ROOT`，从 ROOT/config/config.json 读取配置。首次状态由独立 xssi 安装辅助程序初始化，server 不自动建库，不接受配置或账号 CLI 参数。开发配置显式使用 `development: true` 且 bind 为回环地址，初始密码只经辅助程序 stdin。

开发端到端夹具见 tests/frontend/server.mjs：它在隔离 ROOT 内写入配置，使用离线库适配器显式初始化，再启动本次候选程序。生产安装使用独立 xssi 完整安装包，步骤见[首次安装](../getting-started.md)。

开发目录资源覆盖只接受未绑定源码的开发程序、JSON development=true 及绝对 XCZS_DEV_WEB_DIR；正式绑定程序拒绝覆盖。测试网关保留通用 Host、Origin 和 HTTPS；程序不识别可信代理或转发 IP，认证不按 IP 建频率桶。

默认 ROOT/state 保存 state.sqlite3 和 tags.db，日志 ROOT/logs，临时文件 ROOT/run。共享文件与实例业务状态可以明确外置。管理员、会话和审计统一存入 state.sqlite3 公共表；旧管理员 JSON 和旧数据库结构一律不接受。详见[运行入口](../cli.md)。
