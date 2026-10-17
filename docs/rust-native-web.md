# Rust 原生 Web 构建与测试

生产前端编译、资源整理、gzip、清单、预算/许可证检查和后端集成使用本仓库 `tools/web-build`，不需要 `npm ci`、Node 或 Vite。后端仍为 Axum。前端只使用裁剪的 xcss-web CSR；框架源码与公共组件由 xcss 统一维护与同步。

在产品根目录执行：

```sh
cargo +1.99.0 run --quiet --locked --manifest-path tools/web-build/Cargo.toml -- build --product-root .
cargo +1.99.0 run --quiet --locked --manifest-path tools/web-build/Cargo.toml -- serve --product-root . --debug --bind 127.0.0.1:5173
cargo +1.99.0 run --quiet --locked --manifest-path tools/web-build/Cargo.toml -- server --product-root . --config xcss-web-build.json --mode development
```

需要 Rust 1.99.0、wasm32-unknown-unknown target 和 wasm-bindgen-cli 0.2.129。默认输出 `web/runtime-dist`；`--dist` / `XCSS_WEB_DIST` 用于隔离验收构建。`serve` 监听 Rust/资源文件变化；`preview` 只提供已有资源，不重新构建。Cargo 缓存齐备时可显式传 `--offline`，默认允许获取锁定依赖。

package.json 的 build/dev/check/test 入口调用同一套 Rust 命令。Node/Playwright 只用于当前真实浏览器验收，已删除退役的 Vite 配置、插件及其编译依赖；生产构建不读取 node_modules。历史 React 夹具已从工作树移除，现行行为由 Rust 单测和实际 WASM 浏览器测试验证。

`tools/web-build/test-preview.mjs` 是测试专用适配器：浏览器脚本仍通过 Node 执行，但 HTTP 资源由 Rust CLI `preview` 提供。测试默认申请端口 0，读取 listening JSON 后返回实际端口，并在清理时结束 Rust 子进程。可设置 `XCSS_WEB_BUILD_BIN` 复用预编译工具，避免每个测试重复启动 Cargo。

CI 与正式打包脚本的生产 build/server 直接调用 Cargo，保留原 source revision、receipt、精确安装器与包校验守卫。浏览器测试仍安装其锁定 devDependencies；它们属于验证工具，不属于 Axum 运行时或生产前端构建依赖。

`npm run check:xcss` 校验实际随产品提供的 vendor 字节、字体摘要、不可变 v1 来源以及 Rust/WASM 入口，不导入旧 npm 包，也不要求安装依赖。该检查与退役依赖负向回归已纳入 CI 和发布质量门。`@xcss/web/` 是 Rust 构建器指向 `web/vendor/xcss/` 的资源别名；`SOURCE.json` 保留真实 v1 包 URL、integrity 和 archive 映射，不代表安装或使用 v1 JavaScript 运行时。浏览器字体断言同样直接核对生产 vendor。

语言验收 `npm run test:language` 默认覆盖 Chromium/Firefox 的移动和桌面布局；CI 在每个引擎的第一个 shard 复用已生成资源执行，发布质量门同样运行。可用 `XCZS_LANGUAGE_TEST_BROWSER=chromium` 或 `firefox` 单独诊断。浏览器版本由 Playwright 锁定依赖决定。

浏览器格式检查使用 `cargo +1.99.0 fmt --manifest-path web/xcss-web/Cargo.toml -- --check`，覆盖产品自身 Rust 源码；不要加 `--all` 遍历上游原样随附的 core/macro。共享源码来源由摘要验证，公共源码规范由 xcss 自身质量门负责。

浏览器入口先复用已验证的候选 Server，再在 Playwright 启动前编译 `offline_fixture`；通过 Cargo JSON artifact 获取实际路径，支持自定义 `CARGO_TARGET_DIR`。已有 `XCZS_FRONTEND_FIXTURE_BINARY` 时直接复用。`webServer` 只初始化临时状态并启动实际候选，禁止在 120 秒启动预算内执行 Cargo 冷编译；测试用例超时不变。
