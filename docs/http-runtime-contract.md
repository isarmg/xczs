# Xczs HTTP 与运行时合同

本文描述当前 Xczs 的 HTTP 行为、xcss 分工和验证入口。Rust 与 Web 依赖由
`Cargo.toml`、`Cargo.lock`、`package.json` 和 `package-lock.json` 固定到 xcss 1.0.0。
当前为本地已验证、尚未发布的候选输入，来源及安装条件见[公共依赖](common-support.md)。

## 请求与认证

| 入口或机制 | 当前行为 |
| --- | --- |
| `/healthz`、`/readyz` | 公开最小存活、就绪响应；管理业务接口仍需认证 |
| 管理员认证 | xcss Axum Adapter 处理会话、Cookie、CSRF 和同源校验 |
| 登录与文件页面 | React 渲染页面结构，加载同源模块、共享 UI 和样式 |
| 标签页面与 API | 文件目录表包含标签列，搜索查询 `all`、`any`、`exclude` 与文件名条件同时生效，分页游标绑定完整条件；当前目录地址使用同一管理员会话与应用入口；菜单通过 `#files`、`#tags`、`#status` 路由切换，未知 hash 显示页面不存在且不改写地址；`/api/v1/file-tags/*` 仅向已认证管理员开放，写入仍需 CSRF |
| 请求 ID | xcss 校验或生成 ID，供响应和日志关联 |
| GET/HEAD | 登录、列表和操作查询为 GET-only；文件 HEAD 包含上传状态查询分支 |
| 方法拒绝 | 受保护接口先认证；公开静态资源允许 GET/HEAD，其他方法返回 405 和 Allow |
| 原始路径 | 路由前验证并传递 RoutePath，保持原始 URI；路径策略只解码一次 |
| 保留路径 | 启动时只读检查冲突，发现冲突即拒绝启动 |

业务 JSON 上限为 16 KiB。上传预检上限为 2 MiB、512 条路径、256 KiB 路径字节，并发上限为 4。
PUT/PATCH 使用流式读取与实际字节限制。写入是否进入提交阶段决定超时、取消和恢复语义；
未知结果通过原操作 ID 查询，不能直接重放写入。详见[工作流程](project-workflow.md)。

## 代码边界

xcss 负责通用认证、请求 ID、连接限制、信号和有界停机。
Xczs 负责共享根路径安全、文件流、列表、上传、持久操作和删除回收。管理员与会话保存在 state.sqlite3 公共表，
业务操作状态由 Xczs 的 SQLite actor 管理。标签索引、关联及在线备份由私有状态目录中的独立 `tags.db` 管理。

标签 API 包括 `GET status/files/folders/tags`、`POST scan/backup/tags/file-tags/files/{id}/confirm/files/{id}/relink`，以及 `PUT,DELETE tags/{id}`，路径均位于 `/api/v1/file-tags/` 下。文件下载继续使用 Xczs 原有的受保护文件路径。扫描只在完整成功后更新缺失状态；文件替换和改名不自动继承标签。

标签 ID 在删除后不会被新标签复用，包含清空标签表并重新打开数据库后的情况。旧页面持有的已删除 ID 不能编辑、删除或关联到新标签。当前 `tags.db` 按精确 schema 指纹验证；使用可复用标签 ID 的旧结构将被拒绝打开，不会自动迁移、重置或修改现有数据库。

React 页面中的文件和上传控制器拥有各自 DOM 区域。主题更新保留正在运行的上传队列。
CSP 只允许同源静态资产，SVG 使用同源摘要路径。产品源码接受严格类型检查及 ESLint 浏览器安全检查。
xcss 共同清单绑定 native 与 React 资源的路径、MIME、大小和 SHA-256。正式资源以完整清单摘要
构成 URL 命名空间，GET/HEAD 与条件请求采用同一提供器，304 保持 immutable 缓存策略。开发目录模式
明确使用未绑定 binary，读取最新资源并重新验证 ETag，不使用生产 immutable 缓存。

## 验证入口

通过共同入口构建 Web 与候选二进制，再执行专项检查：

```sh
npm run build:server -- --no-install
npm run check:docs
npm run check:js
npm run check:types
npm run test:frontend:unit
cargo test --locked --target x86_64-unknown-linux-gnu --all-targets --all-features
```

| 行为 | 测试入口 |
| --- | --- |
| 认证、重复安全头、CSRF、GET-only | `tests/auth.rs`、`tests/http_contract.rs`、`tests/browser_api/request_validation.rs` |
| 请求无需来源地址、启动路径冲突 | `tests/library_api.rs`、`tests/platform_namespace.rs` |
| 上传长度、offset、空间和恢复 | `tests/http/upload.rs`、`tests/http/resumable_upload.rs`、`src/server/upload/tests.rs` |
| 操作归属、指纹和持久结果 | `src/server/operation_registry/tests.rs`、`src/server/state_store/tests.rs` |
| 探针、多地址监听和停机 | `tests/health.rs`、`tests/bind.rs`、`tests/shutdown.rs` |
| 标签扫描、筛选、关联和下载边界 | `src/server/tagging/db.rs`、`src/server/tagging/scan.rs`、`tests/tags.rs` |
| 正式运行时和发行包 | `scripts/check-release-runtime.sh`、`scripts/check-formal-release-e2e.sh` |

测试使用独立临时共享根和状态目录。正式发布还要求候选包、完整源码提交、SHA-256 清单和验收结果一致；
具体流程见[运维文档](operations.md)。

目录 API 的 `file_tags` 数组与 `paths` 一一对应，包含 `file_id`、有界 `tags` 和 `tags_has_more`。读取标签会核对当前文件的设备、inode、大小及 mtime/ctime；替换文件不继承旧标签。暂时无法读取标签时 `file_tags` 为 `null`，文件列表仍可操作，页面明确显示标签不可用。`GET /api/v1/file-tags/file?path=相对路径` 以同一身份规则读取单个文件的标签。
