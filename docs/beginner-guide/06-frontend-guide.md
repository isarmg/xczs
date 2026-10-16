# 06. 前端页面与交互：Leptos 浏览器工作区

本章介绍当前 Rust/Leptos 前端。HTTP、文件系统、会话和持久化仍由 Axum 服务处理；浏览器界面、状态和请求编排编译为 WASM，在用户浏览器运行。它采用 CSR，页面交互依然需要浏览器执行代码。

上传的协议与提交边界见[第 7 章](07-upload-protocol.md)，构建和迁移证据见[Leptos 浏览器维护](../leptos-browser.md)。

## 6.1 从 HTML 到工作区

Axum 返回 `web/index.html` 或 `web/login.html`，引用同源 CSS、字体和外置 ES module。HTML 保留 `#xczs-root`、登录密码字节范围和 Base64 编码的业务元数据，不向页面注入管理员凭据。

启动入口 [leptos-index.js](../../web/leptos-index.js) 和 [leptos-login.js](../../web/leptos-login.js) 加载同一个 WASM 模块；[leptos-platform.js](../../web/leptos-platform.js) 导入 xcss 样式、字体许可和现有 server-corners 绘制适配器。剩余手写 JavaScript 只负责这些浏览器边界与只读验收桥接，文件管理和上传状态保存在 Rust 中。

[lib.rs](../../web/leptos/src/lib.rs) 初始化共享字体、语言、主题和挂载根节点。文件页先经共享客户端恢复会话，再严格检查 `index-data` 的字段、路径和类型，最后创建工作区模型。登录页使用服务端提供的密码字节范围，保留必填错误、焦点、密码清空、请求锁定及中英文显示。

真实 Rust 运行时启动后才写入 `html[data-xcss-runtime="leptos"]`。浏览器验收还等待实际 WASM 请求，避免只看页面外观就误用旧实现。

## 6.2 源码地图

| 文件 | 职责 |
| --- | --- |
| [lib.rs](../../web/leptos/src/lib.rs) | 页面启动、会话恢复、登录与只读诊断桥接 |
| [model.rs](../../web/leptos/src/model.rs) | 工作区数据、筛选、目录页、操作模式、上传模型与边界校验 |
| [workspace.rs](../../web/leptos/src/workspace.rs) | 导航、目录列表、搜索、分页、行内编辑、键盘与显示 |
| [operations.rs](../../web/leptos/src/operations.rs) | 新建、重命名、移动、删除、确认对话框与列表失效 |
| [http.rs](../../web/leptos/src/http.rs) | Fetch/XHR、响应预算、结构化错误、Operation/job 查询与路径 |
| [protocol.rs](../../web/leptos/src/protocol.rs) | 上传 status/header 矩阵、身份、长度、offset、revision 与恢复判定 |
| [upload.rs](../../web/leptos/src/upload.rs) | 选择、预检、有界队列、正文、冲突、取消、重试与终态历史 |
| [tags.rs](../../web/leptos/src/tags.rs) | 标签库、分页、文件关联、筛选、索引状态与扫描 |
| [vendor/xcss-web](../../web/leptos/vendor/xcss-web) | 本产品随附的共享认证、UI、主题、语言与合同快照 |

旧 React/TypeScript 源码与业务单测保存在 [迁移基线](../../web/tests/fixtures/react-baseline/README.md)，便于核对历史合同；当前生产构建不从该目录生成界面。

## 6.3 修改源码为什么要重新构建

浏览器 Rust 代码先编译为 WASM，wasm-bindgen 生成浏览器绑定，Vite 收集 JS、WASM、CSS、字体、图标和许可。`scripts/build-platform.mjs` 用私有、URL 安全的临时目录构建，再发布到 `web/dist` 和 `web/runtime-dist`。Axum 构建把运行资源及其清单嵌入可执行文件。

```text
修改 Rust/CSS → npm run build:platform → 编译 Axum → 启动新程序 → 浏览器重新载入
```

生产程序不会每次请求都读取仓库源码。未绑定源码的开发程序可显式启用开发资源目录；正式程序的目录覆盖边界仍按原规则校验。

构建输入包括独立浏览器 `Cargo.lock` 和随产品提供的共享源码。`SOURCE.json` 及 `assets/leptos-build.json` 记录摘要，构建中输入发生变化会拒绝该次产物。运行资源需要匹配清单、MIME、大小及内容摘要。

## 6.4 目录、筛选和分页

目录路径、搜索、排序和标签筛选属于工作区状态。文件列表只为当前窗口创建 DOM；服务端 cursor 继续绑定目录身份、账号、查询和排序，不把浏览器当前行号当成可信后端状态。

新的路径或筛选会建立新一代读取。旧读取需要取消或丢弃过期结果，避免快速切换后让旧目录覆盖新目录。分页不能把另一个查询的结果混进当前列表。首屏目录冲突的自动读取恢复只接受原合同规定的 HTTP/status/code/recovery 组合，并且最多一次。

标签筛选在用户提交前保留本地草稿；选择跨页标签时不能因为当前选项页变化就丢失已选择项。组合筛选、目录返回和导航仍保留 URL/history 语义。

## 6.5 操作和不确定结果

新建文件夹、行内重命名、移动和删除都通过现有受认证接口完成。提交期间锁定相关控件；失败时保留可恢复的编辑状态、确认对话框与焦点。

普通变更携带原 Operation ID。网络错误、超时或不可信成功包络不能证明写入失败，浏览器先查询同一操作的只读状态，不盲目重放。已确认成功、明确拒绝和未知结果分别影响目录刷新或失效提示。

取消配对、上传取消、清理 stage 和关闭对话框是不同操作。上传的取消需要保留当前 Upload ID、清理状态及认证暂停；不能只把一行从 DOM 删除就声称取消成功。

## 6.6 上传状态与浏览器资源

用户选择的 File 只属于当前文档，不持久化文件内容。刷新后重新选择一个同名、同大小和同修改时间的文件，不足以证明它是原来的内容；恢复必须继续核对原上传会话。

默认一个上传并发；单批最多 512 个文件，全页最多 512 个待处理项。尚未生成行的批次尾部也占用容量并参与取消。终态历史保留最近 200 条，并处理正在聚焦的行被淘汰后的焦点回退。

XHR 仍提供上传进度、超时和 abort，但事件回调、队列、冲突策略、HEAD/PATCH/discard 编排都在 Rust。组件离开或任务取消时需要释放监听器、定时器和未完成读取；变更的未知结果按协议继续对账。

## 6.7 类型与运行时验证

Rust 编译器检查本地类型和所有权，无法保证服务器 JSON 或 HTTP 头符合预期。浏览器仍需验证字段、上限、整数规范、唯一身份、状态矩阵和响应预算。外部响应通过验证后才进入业务状态。

运行编译检查：

```sh
npm run check:types
cargo +1.99.0 fmt --manifest-path web/leptos/Cargo.toml --all --check
cargo +1.99.0 clippy --manifest-path web/leptos/Cargo.toml --locked \
  --target wasm32-unknown-unknown --all-targets --no-deps -- -D warnings
```

`check:types` 是保留的命令名，现在检查生产 Rust/WASM 浏览器 crate。

## 6.8 验证与排错

```sh
npm run check:js
npm run check:docs
npm run test:frontend:unit
npm run test:language
npm run test:frontend
```

Rust 合同测试验证协议矩阵、路径、页数据和绑定条件；活跃 Node 单测验证发行、Shell 和浏览器安全工具。Playwright 通过真实 Axum 程序和测试 HTTPS 网关验证原交互、上传、ARIA、键盘、焦点、中英文与移动布局。

排错时先确认 WASM 请求成功、MIME 正确、运行标记已出现，再看会话恢复与业务请求。协议问题结合响应头和服务端 request/operation/upload ID 判断；不要把 Cookie、CSRF Token 或文件正文复制到公开日志。

旧基线测试通过只能证明旧实现，不能替代新 WASM 的真实浏览器验收。实际通过范围以当前验收记录为准，不把框架迁移当作性能收益的实测证据。
