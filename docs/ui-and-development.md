# xczs 界面、操作反馈与开发

技术栈与其他 Server 项目统一：后端为 Rust 2024 / Tokio / Axum 和 SQLx / SQLite；前端为 React 19 / TypeScript / Vite，复用 xcss 的管理员认证、UI、设计令牌和字体。浏览器源码全部使用 `.ts`/`.tsx`，继承 `@xcss/web/web-toolchain/tsconfig.json` 的严格配置；构建先检查类型，再由 Vite 编译所有页面和文件业务模块，最后通过 `xcss::web_assets` 嵌入服务端。

## 界面操作与标签

操作步骤见[文件和标签指南](usage.md)。三个菜单共用 React 应用，使用 `#files`、`#tags`、`#status`；筛选使用 `q`、`all`、`any`、`exclude` 查询条件。导航和浏览器历史保持目录、输入和上传任务。
索引及人工标签位于 `data_dir/tags.db`；扫描和文件身份确认见[运行参考](runtime-reference.md)，服务与权限见[运维](operations.md)。

标签管理提供索引目录浏览和分页入口，可进入目录查看原文件；没有已索引文件的空目录不在这份索引中。打开未索引文件的标签栏只读取状态，不自动扫描；点击明确的“扫描整个文件库”后才开始完整扫描。扫描请求单独允许 90 秒，包括最多 30 秒的目录收集及随后数据库提交，普通标签读取仍为 5 秒。结果未能确认时先查看状态页的上次完整扫描时间和错误信息，扫描写入不会自动重试。切换视图会取消已发出的读取并释放前端串行队列。

## 统一错误反馈

Web 与 API 使用一致的结构化错误和操作 ID；遇到写入结果未知时应先查询操作状态，不要直接重放。字段与恢复流程见[项目工作流程](project-workflow.md)。

## 访问日志

访问日志会脱敏认证信息，并记录请求状态和操作 ID。生产文件权限、轮转、容量与停机刷新要求见[运维文档](operations.md)。

## 开发验证

`npm run build:server -- --no-install` 生成未绑定源码的开发 binary，并验收其内嵌资源。需要前端热更新时，
运行时显式使用 JSON `development: true` 与绝对路径 `XCZS_DEV_WEB_DIR` 指向 `web/runtime-dist`；重新执行
`npm run build:platform` 即可更新资源，无需重新编译 Rust。源码绑定的正式 binary 拒绝目录覆盖。
目录模式使用共同 ETag 和重新验证缓存；正式资源路径绑定完整清单摘要，使用 immutable 缓存。

```sh
npm run build:server -- --no-install
cargo fmt --all -- --check
cargo clippy --locked --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo test --locked --target x86_64-unknown-linux-gnu
npm run check:docs
npm run check:js
npm run check:types
npm run test:frontend:unit
```

代码采用 [Apache License 2.0](../LICENSE-APACHE)。
