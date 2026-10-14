# 第 4 章：后端请求生命周期

本章对应 Axum 与 xcss 1.0.0 的唯一服务链路。HTTP 行为和验证入口见[HTTP 与运行时合同](../http-runtime-contract.md)。

## 4.1 总体关系

```text
xcss BoundListeners → 受限 HTTP/1 连接
  → 一次 request ID，不读取或记录访客地址
  → Xczs 原始 URI PathPolicy、RequestProfile、请求许可
  → Axum Router → xcss Axum 认证（受保护路由）
  → 具体业务 Handler → RootedFs / 上传 / 操作登记表
  → 流式 Axum Body → 完成、错误或丢弃时记录访问日志
```

Hyper 只在 xcss 内实现 HTTP/1 传输，不承担 Xczs 的路由分派。产品没有第二套 HTTP 服务入口、信号处理或框架切换开关。

## 4.2 启动顺序

[main.rs](../../src/main.rs) 解析并验证配置，安装平台信号处理，使用 `BoundListeners` 绑定全部地址。后一个地址失败时释放已绑定地址，尚未打开或修改产品状态。

随后在受控阻塞工作中构建产品：先只读检查平台保留路径冲突，再取得共享根锁、验证 openat2、打开严格当前身份的状态库、恢复操作、启动维护。构建完成才交给 xcss 的唯一 `serve(HttpServer, service)` 接受请求。状态身份包含持久化结构版本；不修改不匹配的 metadata。

## 4.3 产品与平台的所有权

[server.rs](../../src/server.rs) 的产品 `ServerRuntime` 只持有业务服务、普通任务和提交任务，作为平台 `LifecycleParticipant` 提供排空与状态关闭回调。xcss `ServerRuntime` 独占连接、信号、健康检查和停机硬期限。

管理员凭据、会话与审计使用 xcss SQLite Store，统一保存在 state.sqlite3 公共表；会话可跨正常重启恢复，受当前过期与撤销规则限制。业务上传状态仍由产品 SQLite actor 管理。

## 4.4 连接边界

全部监听地址共享连接许可。先等某个 listener 真正可读，再取得许可，最后 accept；空闲地址不会预占许可。许可跟随实际 socket，包括升级后的连接，而不是随 Handler 返回而释放。

请求头默认最多等待 10 秒，接收缓冲最多 64 KiB；socket 写入无进展时限 30 秒。接收错误分类记录并有界退避，停机可立即打断。Xczs 不提供 h2c 或 WebSocket 路由。产品不读取 socket 对端和转发地址头，连接许可与业务限制均不以 IP 为键。

## 4.5 路由前的请求边界

[router.rs](../../src/server/router.rs) 的服务包装器在 Axum 匹配前读取原始 URI，调用一次 `PathPolicy`。不重写 URI，也不使用自动解码的 `Path<String>` 作为文件路径。

无效原始路径返回 400，且不会进入文件操作。xcss 在外层只验证或生成一次请求 ID，并写入请求、响应与日志；库调用服务时也不需要地址注入。

## 4.6 RequestProfile

[request.rs](../../src/server/router/request.rs) 一次记录上传、内部 API、认证端点、公共资源及操作 ID。绑定的 `MutationProgress` 在超时后仍能区分未开始和结果未知；外层错误不编造副作用结论。

## 4.7 Axum 路由表

[routes.rs](../../src/server/router/routes.rs) 分别注册公共路由、受认证内部 API 和文件方法。未知内部路径由内部 404 Handler 终结，不会作为共享文件访问。

| 路由 | 方法 |
| --- | --- |
| /healthz、/readyz | GET、HEAD |
| /__xczs__/login | GET；HEAD 明确拒绝 |
| 内容寻址静态资源 | GET、HEAD；其他方法 405 |
| /api/v1/auth/login、logout | xcss POST |
| /api/v1/auth/session | xcss GET |
| /__xczs__/api/list、jobs/{id} | GET；HEAD 明确拒绝 |
| mkdir、move、rename、upload/preflight、upload/discard | POST |
| 共享文件路径 | GET、HEAD、PUT、PATCH、DELETE |

## 4.8 公共路由

`/healthz` 正常返回 204 空响应；`/readyz` 只返回 `{"ready":true}` 或 503 的 `{"ready":false}`。两者都禁止缓存、无需登录、不泄露根路径、账号或错误细节。平台定期刷新真实根可写、空间和 SQLite 探针，不能把在线当成所有业务操作可成功。

登录 HTML 与资源仍由产品嵌入。登录 POST 由 xcss 严格限制正文、同源请求和计算并发。内容资源保持摘要寻址和 immutable 缓存，失败响应不缓存。

## 4.9 认证与页面跳转

受保护路由只调用一次 xcss Axum Adapter，验证会话后传递 `FilePrincipal`。匿名 API 保留平台 JSON 错误；仅合法 HTML 页面导航可以 303 到登录页。平台认证响应不经过产品重新序列化。

## 4.10 CSRF

xcss 校验写方法的唯一 Origin、Host、Fetch Metadata、Cookie 和 CSRF token。重复、拼接、缺失或非规范安全头均被拒绝。认证失败发生在任何文件修改之前。

## 4.11 内部业务 API

JSON 修改仍受 16 KiB 读取预算约束。上传预检独立保留 2 MiB、512 条路径、256 KiB 路径字节和并发 4 的上限，不安装全局 BodyLimitLayer 去限制大文件上传。

## 4.12 文件 Handler

[files.rs](../../src/server/router/files.rs) 的 `FileRequest` 只执行 Axum 已选择的文件动作：读取、上传、续传或删除。它保留协议头解析、路径租约、空间预留和 RootedFs 检查，不重新实现认证或完整 URL 路由。

平台保留路径不可成为文件目标；移动、重命名或删除其祖先同样拒绝。`/api/notes.txt` 等不相关普通文件仍可使用。启动发现真实同名文件时明确拒绝，不删除或静默隐藏数据。

## 4.13 列表

HTML 目录页和分页列表是两次独立请求。列表快照、owner 隔离、排序、分页预算和递归扫描许可不变。目录页不把会话或 CSRF 嵌入业务模板，会话由 xcss Web Client 单独恢复。

## 4.14 流式下载与 HEAD

[download.rs](../../src/server/download.rs) 经 RootedFs 打开真实 fd，保留条件请求和单段 Range。读取按固定块分配，不能随文件大小整体收集。源文件下一块读取的 30 秒时限和 socket 写入空闲时限是不同边界。

文件 HEAD 仍先判断上传状态查询，不误执行普通下载。日志 Body 包装继续转发 frame、错误、size_hint、结束状态和 Drop，不能在 Handler 返回时误记完整下载成功。

## 4.15 上传与文件修改

PUT/PATCH 使用 Axum Body 数据流，按实际字节验证长度、偏移、总时限和空闲时限。修改保留未完成检查点、原子发布与 fsync 顺序。目录创建、移动、重命名和删除沿用已有 owner、操作 ID、指纹和持久操作表。

相同 ID 不同内容返回冲突；响应丢失后查询既有结果。不能用取消等待者的方式取消已经开始的阻塞文件工作。

## 4.16 `MutationProgress`：超时时如何避免撒谎

`PREFLIGHT` 表示尚未进入副作用；`RESERVED` 表示登记但尚未开始提交；`DETACHED_COMMIT` 表示已有必须收尾的提交义务。上传首次修改与超时原子竞争，超时先赢会关闭修改边界；提交先赢时返回结果未知并要求查询，不能报告未执行。

任务登记不等于已产生副作用，客户端断开也不等于提交失败。具体文件与恢复规则见[第 5 章](05-filesystem-state-and-reliability.md)及[上传协议](07-upload-protocol.md)。

## 4.17 错误、缓存与访问日志

平台认证和请求 ID 错误保留 xcss ErrorEnvelope；文件业务错误沿用 Problem Details、操作头和上传恢复字段。默认访问日志增加 request ID，仍对敏感字段脱敏、转义和限长。下载完成状态在 Body 结束、失败或丢弃时确定。

## 4.18 停机顺序

停止接受连接并拒绝新业务 → 同时推进连接、请求与平台后台探针排空 → 关闭普通工作登记并等其结束 → 关闭提交登记并等提交结束 → 关闭 StateStore。

平台健康探针可能持有产品请求许可，所以不能先等待产品闸门、再继续执行被暂停的探针。达到宽限期时只取消可取消工作；硬期限仍未结束则报告未完成连接、工作与提交数，在可执行程序边界非零退出，保留仍在使用的状态所有者直到退出。

release 保持 `panic=abort`，不能宣称 catch_unwind 能隔离 release panic。必须使用真实 release 二进制验证异常退出后的当前状态恢复。

## 4.19 验证入口

```bash
cargo test --locked --all-targets --all-features
cargo test --locked --test http_contract --test library_api
cargo test --locked --test bind --test shutdown --test health
```

完整门禁、浏览器、部署与正式制品验收另见[测试工作流](08-testing-debugging-and-change-workflow.md)。不要把编译成功或 Tower oneshot 当作真实 socket、信号和发行验证。
