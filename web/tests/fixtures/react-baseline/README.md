# React / TypeScript 迁移基线

这里保存迁移前的浏览器源码和 17 份业务单元测试，原始文件字节及断言未删减，摘要记录在 `SOURCE.json`。它们用于源码对照和回归合同查阅，不属于当前生产入口，也不作为 Leptos 运行时通过的证据。生产实现与门禁见仓库的 `docs/leptos-browser.md`。

若需独立重现旧业务单元测试，在本目录执行 `npm ci --ignore-scripts` 和 `npm run test:business`；该目录使用自己的锁文件。根仓库只运行当前 Rust 合同测试及原真实 Axum 浏览器验收。与旧实现无关的 Shell、ESLint 和发行工具测试仍在根仓库的活跃测试目录。

迁移前的 15 份浏览器 spec 与 fixture/runner 支持文件也按 HEAD 原字节保存在 `tests/frontend`，摘要单列为 `browser_contracts`。它们保留旧 JS manager 的历史焦点等断言，供合同查阅；不计入已运行的 71 项历史业务单测，也不会被当前 Playwright 测试目录发现。当前浏览器验收直接运行产品 Rust/WASM，保持对应的数量、焦点、键盘和网络断言。

`legacy_tooling` 另保留原 ESLint 配置、TS/TSX processor 及其 3 项工具合同，包含旧网络 owner 例外的完整断言。这 3 项不计入前述 71 项业务单测。当前工具测试改为核对真实 Leptos JS 引导文件及 Rust 独占网络逻辑的边界。

旧 ESLint 的 3 项工具测试可另用 `npm run test:tooling` 运行；`npm test` 顺序执行业务基线和该工具基线。
