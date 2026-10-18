# xcss-web 浏览器前端

当前入口为 `web/xcss-web`，仅通过 `xcss-web` 路径 crate 接入；主分支使用 Rust 生产构建。构建与验收命令见 [Rust 原生 Web 构建与测试](rust-native-web.md)。本文保留首次迁移时的流程和测量记录，旧 Node/Vite 命令与体积数字不代表本次改名后的验证结果。

文件列表、文件操作、上传、标签库和登录页面的 Rust 实现在 `web/xcss-web`。它使用 xcss-web CSR，编译为在浏览器内运行的 WebAssembly；HTTP 路由、认证、文件系统、SQLite 标签索引和静态文件服务继续由 Axum 提供，保留兼容的既有 API，并按 WASM 加载、缓存和传输需要配合优化。

当前 `main` 已合并 xcss-web CSR 与 Rust 原生构建；直接在主分支构建和验收。下面的首次迁移流程与数字作为历史记录，正式发行和线上部署继续使用产品发布流程。

默认构建只加载 Rust/WASM，直接执行原有构建命令即可。生产 NPM 依赖和锁文件不再包含 React。退役的 React/TypeScript 源码、模拟测试及独立锁文件已从工作树移除，原始迁移对照可通过 Git 历史查阅；当前验收保留 Rust 合同单测和真实 Axum 浏览器断言。

## 构建

先执行 `bash scripts/install-xcss-web-tools.sh` 准备浏览器编译工具。所需版本由 `tools/web-build` Rust CLI 固定：Rust 1.99.0、`wasm32-unknown-unknown` target，以及 wasm-bindgen-cli 0.2.129。Rust 资源构建器负责收集 CSS、字体、许可、SVG 遮罩和发布清单；Node/Playwright 用于浏览器验证。

```sh
npm run build:platform
cargo build --locked
```

如果 wasm-bindgen 不在 PATH 中，可通过 `WASM_BINDGEN` 指定其可执行文件；`CARGO_TARGET_DIR` 可用于共享 Cargo 缓存。离线构建设置 `CARGO_NET_OFFLINE=true`。日常验收使用优化后的默认 release 构建，`XCZS_XCSS_WEB_DEBUG=1` 只用于调试。

共享 Rust 浏览器库已经作为快照放在 `web/xcss-web/vendor/xcss-web`，因此构建不依赖相邻的 xcss checkout。`SOURCE.json` 和浏览器构建清单校验这些输入及生成的 JS/WASM；编译期间修改 Rust 输入会拒绝发布该次产物，需要再次构建。

## 状态与协议

- 浏览器文件句柄只保存在当前文档内。刷新后不会因为同名、同大小和同修改时间而自动恢复另一份文件内容。
- 上传继续使用原有 PUT、HEAD、PATCH、discard 和 preflight 接口。成功、续传、覆盖或重建上传会话，都必须满足原有标识、长度、偏移和目标 revision 的绑定规则。
- 所有变更保持原来的 CSRF、操作标识和未知结果处理。变更请求不进行盲目重放；普通文件操作最多查询一次只读 job 状态，上传根据服务器明确给出的 recovery 执行恢复。
- 单批最多 512 个文件、全页最多 512 个待处理上传、默认一个并发上传和最近 200 条终态历史继续生效。尚未生成 DOM 的批次尾部同样占用容量并参与取消。
- 原来的 CSS、ARIA、键盘和服务端 HTML 数据接口保留。剩余的手写 JavaScript 负责 WASM 初始化、静态资源导入和只读合同测试桥接；共享轮廓的几何、SVG 与 DOM 生命周期由 Rust `mount_server` 接入。

## 验证

Rust 的协议矩阵、页数据校验、路径边界、job 响应绑定和上传历史阈值通过 `cargo test --manifest-path web/xcss-web/Cargo.toml --locked` 检查。浏览器合同测试应对实际加载的 WASM 运行：

```sh
npm run test:frontend
```

测试同时检查运行时的 `data-xcss-runtime="xcss-web"` 标记及实际 WASM 请求。原先直接导入 TypeScript 内部函数的 job 查询用例，改为调用变更流程共用的 Rust 查询函数；历史焦点用例使用真实产品的 200/201/202 行触发淘汰，并保留数量、DOM、焦点与 tabindex 断言。旧的可配置 JS manager fixture 不会冒充迁移后的运行时。

功能验收使用临时回环服务；迁移分支的提交与推送不替换已安装服务，部署仍按正式发行流程进行。

## Axum 静态资源适配

WASM 在构建时生成 gzip 副本，两个版本都进入既有内容摘要清单。Axum 根据 `Accept-Encoding` 提供原始或 gzip 字节，始终保持 `application/wasm`，并通过 `Vary: Accept-Encoding` 和变体独立 ETag 保持缓存隔离；HEAD 保持所选版本长度，条件请求仍返回 304。明确的 `gzip;q=0` 不会被 wildcard 覆盖，缺少压缩副本时回退原资源。开发目录继续直接读取原文件，不让旧 gzip 遮蔽本地修改。

登录和文件页从实际内嵌资源清单生成同源 WASM preload。真实 Axum 静态资源回归检查两版精确清单字节、MIME、nosniff、缓存、HEAD、304、排除协商和不存在路径，协议适配器另有独立测试。该改动只优化静态传输；没有将体积变化当作页面速度的实测收益。

依赖审计覆盖后端与浏览器两份 Cargo.lock，均保持原有 RustSec 和 yanked 拒绝策略；隔离质量门复用同一封存数据库和禁止联网的审计参数。发布包保留现有 SBOM，浏览器锁定包、来源和校验和另作为 `assets/xcss-web-build.json` 的 `cargo_packages` 补充清单随静态资源内嵌。

## 可复查的验收记录

迁移前的 17 份业务单测已独立运行，71 项测试通过；这是历史基线，不能证明当前 Rust 实现。当前 Rust 合同测试覆盖页数据、操作与上传协议、job 绑定、未知结果、历史容量等，工具侧继续运行原有 Shell 和发布策略测试。ESLint 的 3 项活跃测试改为直接核对三份生产 JS 引导文件的注入、动态代码和网络边界，原 TS/TSX 工具合同及配置完整归档并另行运行通过。真实浏览器验收使用内嵌最新静态资源的 Axum 二进制，同时核对 WASM 请求和运行时标记。

真实 Axum 资产测试 3 项和通用 gzip 协商测试 3 项通过。原生 `auth` 20、`browser_api` 35、`http` 65、`library_api` 10、`protocol` 2，共 132 项直接运行通过；HTTP 测试的精确 CSP 期望只补入浏览器执行 WASM 必需的 `wasm-unsafe-eval`，其余防护值和断言保留。CI 的 required check 名称、分片覆盖、失败传播和发布写权限边界均保留。构建器另外检查源码快照、两份锁文件、字体与许可、资源摘要和发布清单，WASM 及预 gzip 字节没有被当作页面性能实测。

2026-10-16 的最新运行时完整 Chromium 验收为 146/146，通过时未使用重试。Firefox 保留迭代覆盖记录：前一完整 145 项运行通过 142 项，剩余为大目录旧错误文本、Retry-After 虚拟时钟竞态，以及并发构建负载下的 129 文件串行上传时限；最新运行时的受影响 28 项回归通过 27 项，唯一标签取消时限随后在隔离环境复验通过。最终隔离 5/5 通过，129 文件串行的 30 秒、标签取消的 1500 毫秒预算均未放宽；其余 3 项只固定测试中的虚拟时间并显式推进时间，生产等待与重试逻辑不因该 fixture 修复改变。本轮没有把这些迭代结果声称为单次 Firefox 146 项全绿。

切换默认入口后重新执行 `npm ci --offline` 和普通 `npm run build:platform`，实际安装树没有 React、ReactDOM、React 类型、React Vite 插件或 TypeScript。WASM 1,316,200 字节、预 gzip 385,194 字节及来源清单，与完整 146 项验收的第四版候选逐字节 SHA-256 相同，gzip 解压等于 WASM。活跃工具测试 12/12、独立性、34 份文档链接、构建器 3 项、前端原生合同 18 项、前后端 rustfmt 与严格 Clippy 均通过。

默认入口额外在 Chromium 和 Firefox 各运行 7/7 必要回归，强制真实 Rust 运行时及 WASM 请求，核对当前 Rust、共享快照和构建器的源码摘要、编译清单与实际 HTTP WASM 摘要，同时验证 Rust job 桥和 4 项上传清理合同。默认语言脚本在两引擎通过，包括禁用存储时显式语言跨登录跳转保留。旧 React 分支和退役模块 fallback 已从活跃浏览器测试中移除。服务端 workspace 的 rustfmt 与 `cargo clippy --locked --all-targets --all-features -- -D warnings` 通过。

最后一次默认模板/二进制重嵌入后的原生回归为认证 20/20、静态资源 3/3、真实 socket 录制 HTTP 合同 1/1（包含 22 个录制场景），全部通过。Renderer 期望改为 xcss-web，原认证、Cookie/CSRF、HTTP、文件变化及操作状态断言继续保留。
