# xczs 持久状态与运行参考

本文供状态机、HTTP 和运维开发使用。文件操作见[使用指南](usage.md)，主机配置见[运维](operations.md)。

## 统一状态库

服务只使用文件型 SQLite schema revision 1 的统一 state store，其中有 `operations`、`upload_sessions` 和 `purge_jobs` 三类状态。SQLite 是上传状态的唯一权威，服务不会在共享根内写入、读取或导入 JSON 上传状态文件。JSON `data_dir` 默认 ROOT/state，也可显式提供业务状态目录，数据库固定使用 `<data_dir>/state.sqlite3`；没有进程内数据库或单独文件路径配置入口。

operation 容量为全局 4096、每账号 1024，终态 TTL 为 15 分钟。启动恢复会删除未进入提交边界的 `Reserved`，把可能已经触碰文件系统的 operation `CommitStarted` 转为 `Completed/unknown` 并从恢复时开始新的 15 分钟终态 TTL；未过期的原有 `Completed` 只继续使用剩余 TTL。upload session 容量为全局 16384、每账号 4096，每次实际更新后保留 7 天；持久状态包含 `Running/CommitStarted/AwaitingConfirmation/Committed/Rejected/Unknown`，重启会把 upload `CommitStarted` 恢复为 `Unknown`，而 `AwaitingConfirmation` 保留完整 stage 等待明确发布或丢弃。首次 discard 原位写入 `Rejected` 并设置终态 TTL；对已有 `Rejected` 的幂等重试不写库、不续 TTL，只继续 identity-safe stage 清理。purge job 容量为全局 4096、每账号 1024，不使用 TTL 或固定失败次数丢弃普通 I/O 故障中的回收任务。

认证客户端应通过 `GET /__xczs__/api/jobs/<UUID>` 查询当前账号的 mutation job。响应使用 `job_id` 字段及 `running/succeeded/failed/unknown` 状态。

状态库固定使用 SQLite rollback journal `DELETE` 模式和 `synchronous=EXTRA`，由单独状态线程串行访问。数据库文件以 `0600` 使用，必须位于共享根之外；已有数据库还必须是非符号链接、单硬链接普通文件，并绑定创建时共享根的设备号和 inode。任何 SQLite 连接打开前，固定 `-journal/-wal/-shm` 都要经 `lstat`、`O_NOFOLLOW|O_NONBLOCK` 打开、`fstat` 和打开前后身份复核，拒绝符号链接、特殊文件、多硬链接、出现/消失或替换；主库不存在时不接受任何孤立 sidecar。现存主库和 WAL/journal 经公共稳定快照机制只读复制到私有临时目录，先验证精确的五列 `product_metadata`、`xczs` 应用名、当前持久化结构身份 `xczs-db-v1`、schema revision 1、xcss 规范 SHA-256 指纹、根绑定和完整性，再打开原路径的当前业务连接。指纹对排除 `sqlite_*` 与 `product_metadata` 后按 `type/name/tbl_name/sql` 排序的原始字段逐个编码 u64 大端长度和字段字节。只有显式 `init` 会在新文件中创建唯一当前 schema；任何非当前 identity、无标记库、版本/指纹/对象漂移、其他应用数据库、错误共享根或非 SQLite 文件都会在 chmod、journal mode 和恢复写入前拒绝，主库及全部 sidecar 保持原字节、mode 和身份。同一状态库不能复制给另一共享根复用。

SQLite 提交与共享根中的 mkdir、rename、文件同步和目录 `fsync` 不属于一个共同事务。operation/upload 崩溃恢复中的 `unknown` 是保守结果，不是回滚记录。DELETE 先持久化含根内相对目标/trash 路径和源 dev/inode/类型的 `Prepared` outbox，再做 checked rename 与父目录 `fsync`；成功后才把覆盖 dev/inode、类型、链接数、大小、uid/gid、完整 mode 和纳秒级 mtime/ctime 的 32 字节 trash revision 与 `Ready` 原子写入。worker 把到期 job 原子 claim 为 `Claimed`，并用 revision 与持续 fd 锚点共同复核；普通 I/O 失败持久化回 `Ready` 并从 100 ms 指数退避到最长 30 秒。若 state-store 的 defer/complete 命令瞬时失败，worker 会有界保留本地 claim，并在回读确认数据库仍为 `Claimed` 后重试；重启也会把遗留 `Claimed` 恢复为 `Ready`。`Prepared` 没有已提交 revision，reconciler 始终保留目标，把 trash 路径上的任何 occupant 移入 `.xczs-quarantine-<uuid>.hold` 后释放 intent，绝不再依据弱源 inode 推断 rename 结果。`Ready/Claimed` 缺失 revision、身份不匹配或最终删除出现 `InvalidData` 时同样 quarantine 整棵 trash 根并释放 job。每个最终 unlink/rmdir 候选先移入随机隔离名，再用既有 fd 复核；`ENOTEMPTY/EXIST` 等异常不从 cursor 0 重扫。DFS 最多保留 2048 层目录 frame，每次 push 都用 `try_reserve`；超深树返回 `InvalidData` 并把剩余 trash 根永久隔离，内存预留失败则保留游标供以后重试。未记账 orphan 在兜底通道满、取消或普通 I/O 失败时保持隐藏，等待以后 maintenance 重新发现；若 purge 判定为 `InvalidData`，整棵根立即进入永久 quarantine。quarantine 永不由 maintenance 自动清理；发现后应停止 Xczs，核对内容、owner、来源日志和状态库再手工移除。递归清理不会进入 trash 下的嵌套/bind mount，普通 mount 边界 I/O 故障保留 job 并退避，卸载后继续。能用 inotify 竞争随机隔离名的恶意同 UID writer 仍不在支持边界内。

进程若在嵌套候选已改成随机隔离名、尚未 unlink 时中断，下一次 orphan 扫描可以重新捕获外层 trash，但 purge 一旦看见树内遗留的隔离名就按身份安全异常停止，并 quarantine 整棵外层根。不要手工把该嵌套名称改回普通文件名后继续运行；应按整根 quarantine 的调查流程处理。

不要在活跃 upload 或未完成 purge 的路径祖先上依赖 rename/unlink 来“顺手迁移”控制状态：SQLite 与文件系统无法在一个事务内原子 rebase。服务会在语义路径租约内，对 move/rename 的源与派生目标、DELETE 目标和 fresh PUT 目标执行有界 keyset 状态检查；根内符号链接别名也按目录身份识别。命中时分别返回 `409 move_state_conflict`、`409 rename_state_conflict`、`409 delete_state_conflict` 或 `409 upload_state_conflict`，待原任务完成后用新的 operation/upload ID 重试。检查本身暂不可用时不会开始 mutation，并返回带恢复建议的 `503`；fresh PUT 的该检查受 upload deadline 约束，超时返回绑定的 `408 not-started`。它发生在 tracked route metadata 之后、注册上传 mutation 和创建 stage/SQLite 行之前。

上传任务是否可能产生未知结果，取决于是否通过首次 mutation 边界。任务可以在持有路径租约和上传槽时只读查询 owner 会话、目标 identity/metadata 与空间状态；创建祖先或 stage、截断既有 stage、更新 SQLite 会话或接收正文等首次 mutation 必须先通过一个与总 deadline 原子竞争的边界。deadline 先赢会关闭边界并 abort 任务，返回绑定的 `408 request_timeout + not-started + retry`；只读准备中逸出的超时类错误同样返回 `408`，其他未处理 I/O 返回 `503 upload_precommit_failed + not-started + retry`。边界关闭后任务不能稍后恢复并写入。若 mutation 先赢，随后外层 deadline 或未处理错误才是 `unknown + query_upload`。运维自动化不要只按 HTTP `408/503` 重放；仍应遵守响应中的 upload state/recovery，并以原 ID 做 owner-scoped HEAD，因为 `not-started` 不排除更早请求留下的检查点或终态。

维护脚本的只读 `check` 通过独立 xssi 辅助程序打开经过验证的私有快照，不打开原路径的业务连接；该入口要求写入服务已停止。在线写入导致主库或辅助文件的正常数据代际变化时，本次诊断仍失败，并返回可重试的 `snapshot.source_changed`；活动写事务返回 `snapshot.busy`。等待写入停止后重新执行检查即可，不能通过删除数据库处理这类瞬时错误。文件替换、权限和链接异常仍受身份检查约束；启动及修改路径继续使用严格复核。错误记录和命令作用见[当前服务命令](cli.md)。

## 上传预检与条件覆盖

首方浏览器在批次入队前向 `POST /__xczs__/api/upload/preflight` 提交最终绝对逻辑路径。一次请求必须包含 1～512 个互不重复的路径，解码后的路径 UTF-8 总量最多 256 KiB，JSON wire body 最多 2 MiB；响应按原顺序返回 `path`、`exists`、`replaceable` 和可选 `revision`。没有已存在目标时不会弹出确认；已存在且可替换的文件才进入覆盖/跳过/取消对话框，不能替换的目标不会自动覆盖。预检是有界观察，不是锁定文件系统的事务；提交时仍须执行 no-replace 或 identity 条件检查。

所有 PUT/PATCH 都使用明确的覆盖策略。缺少 `X-Xczs-Upload-Overwrite` 或值为 `false` 表示原子 no-replace；目标在提交时存在就失败，不会静默覆盖，rename 成功后还会核对目的名称与已打开 stage 的 identity，无法证明时返回 unknown。值为 `true` 时必须同时提供 64 位小写十六进制 `X-Xczs-Target-Revision`。revision 绑定账号摘要、规范根内目标路径和完整 replacement identity，服务在真正 rename 前再次验证它；但已有目标覆盖随后使用普通 rename，不是能排除共享根外部 writer 的原子目录项 CAS。客户端不得把网络错误、`unknown`、非法响应或无法解析的 revision 降级为无条件覆盖，生产运行期间也不得由其他进程并发写共享根。

若完整 stage 在最后的条件检查中遇到目标出现或变化，服务保留同一 upload ID 和满 offset，并持久化为 `AwaitingConfirmation`；HEAD/冲突响应对外使用 `awaiting-confirmation`，同时给出当前 revision 和可替换提示。用户接受最新目标后，浏览器以同一 ID、满 offset、空正文 PATCH 和最新 revision 再次提交，因此通常无需重传文件；目标若再次变化会继续失败关闭并重新确认。每一个可信 target-change 都重新发出 `refresh-required`，不会因为该 uploader 先前已使列表失效就吞掉通知；两次冲突间点 Refresh 得到的新 snapshot 也会再次失效。用户跳过时，浏览器向 `POST /__xczs__/api/upload/discard` 提交同一路径和 upload ID。服务先以 owner/path/ID 绑定的原位 CAS 把 `AwaitingConfirmation` 持久化为 `Rejected`，再按已记录 stage identity 条件清理；已有 `Rejected` 的重试不续 TTL，但会继续清理。成功 `204` 表示终态已确定且本次安全清理步骤完成，可能是原 inode 已删除、已经不存在，或发现同名替换物并保留；仅由 HEAD 得到 `rejected` 只证明上传未发布和 discard 决定已持久化，不证明路径物理消失。

每个目标父目录下的 stage 都放在精确名为 `.xczs-upload-stages` 的当前私有目录中。该目录必须是服务账号所有、真实目录、与目标父目录同一设备且精确为 `0700`；stage 初建为 `0600`。覆盖提交重放目标 mode/ACL/xattr 后，stage 本身可能不再是 `0600`，但父目录仍阻止其他本机账号遍历和读取未发布内容。启动在监听前以 16 行 keyset 页验证所有数据库记录只引用这一当前布局，并核对目录权限、owner、设备和活跃 stage inode；任何其他持久 stage 路径都会失败关闭且不会移动文件或改写数据库。

有一个必须保留的 metadata 安全例外：已暂存的覆盖上传可能已经重放旧目标 uid/gid、mode 或允许的 xattr。若旧目标随后消失，服务以 `upload_metadata_preservation_refused` 拒绝用空 PATCH 把该 stage 当作全新文件发布；浏览器必须先 discard，再生成新 ID，以完整正文和 create-only PUT 重传。这样避免把旧对象的 metadata 意外赋给一个语义上新建的文件。

仓库 systemd 样例的 `TimeoutStopSec=120s` 大于应用内置停机边界。首次信号停止准入，普通工作和已登记提交共用 30 秒宽限；宽限耗尽或收到第二个信号后，取消普通工作，再给排空 10 秒。第三个信号或强制阶段超时会结束排空并报告失败。成功与失败都调用日志刷新，最多等待 5 秒，然后显式以状态码 `0` 或 `1` 退出，不通过 runtime Drop 等待卡在内核或 FUSE 的任务。日志写入使用独立 OS 线程，刷新调用者只在有界时间内等待确认，期间没有另一个停止信号监听器。失败不能保证未完成提交或尾部日志落盘；调大 systemd 超时不会延长这些应用期限。应演练最慢文件和目录同步，确保正常提交能在窗口内完成。SIGKILL 会绕过应用停机代码。

## 请求与网关

自行管理的 HTTPS 网关应固定规范 Host，关闭写请求重放与缓存，并对登录入口独立设置速率、连接与正文预算。Xczs 不附带代理软件配置。内部路由只接受规范 URI，尾斜杠、重复斜杠和非规范百分号编码不会成为另一个登录入口。

xcss 统一限制登录正文为 16 KiB、读取期限 10 秒、全局 32 个读取许可；取消或失败释放许可。最多两个 Argon2id 计算槽，取得计算槽最多等待两秒，许可持续到实际计算结束。没有 IP、账号或会话失败频率桶，容量已满或超时按当前认证资源错误返回。


Xczs 的普通文件和 Range 正文没有总时长/最低速率限制，但每个源文件分块的门控等待及读取连续 30 秒未完成会使正文报错，已经取得的分块在套接字连续 30 秒没有写入进展也会关闭连接。两项 idle deadline 独立重置；公网网关仍应设置符合业务容量的响应总时长、最低速率和空闲策略，不能把它们当作完整的慢客户端或总时长防护。

后端必须由主机防火墙或网络 ACL 限制为仅网关可达。回环端口不区分网关 和其他本机进程；不可信本机进程必须隔离。TLS 私钥、Cookie、CSRF、完整 PHC 和文件内容不得进入诊断工单或公开日志。

## 健康检查

- `GET` 或 `HEAD /healthz` 是公开 liveness，只表明进程仍能处理 HTTP，不访问文件内容，也不泄露账号或路径。
- `GET` 或 `HEAD /readyz` 无需认证，只有最小 `ready` 布尔值。xcss 在启动和每 5 秒刷新共享根私有隐藏探针的写入、同步与读回、SQLite 回滚写事务及最低空间探针；探针文件在开始监听前固定创建，周期检查不改变共享根目录时间戳。每次探针有平台时限，失败返回 503，停止准入立即变为未就绪。端点读取最近一次结果，不泄露路径、账号或错误详情，也不是对 rename 和所有业务配额的完整保证。
- 告警至少覆盖进程重启、HTTP 5xx/429/507、认证资源容量、磁盘空间、inode、共享根挂载状态、备份年龄和备份恢复演练结果。

普通写请求返回成功只表示其规定的原子发布和目录同步步骤已返回成功。硬件、固件、网络文件系统或宿主机错误兑现同步请求仍可能破坏数据，因此监控不能替代备份。

## 公共接口标识

实例锁和原子临时文件遵守项目根目录《规范》附录 A 的公共运行接口约定；安装维护工具的协调职责见[运维](operations.md)。服务身份头为 `x-service`，健康状态中的公共源码修订字段为 `common_revision`。管理会话采用 `__Host-admin-xczs-session`，显式开发模式采用 `admin-xczs-session`；生产 Cookie 的 Secure、HttpOnly、SameSite、Path 和 CSRF 约束继续生效。资源清单格式为 `web-assets-v1`；state.sqlite3 使用组合后的当前产品 DDL，包含公共管理员、会话与安全审计表；tags.db 保持独立的标签 DDL。
