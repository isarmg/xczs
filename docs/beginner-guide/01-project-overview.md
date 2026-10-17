# 01. 项目全景：先知道自己在读什么

## 1.1 一句话理解 Xczs

Xczs 是一个只运行在 Linux AMD64 GNU（`x86_64-unknown-linux-gnu`）上的浏览器文件管理器：服务端把某个目录设为“共享根”，认证后的管理员可以通过网页浏览、搜索、上传、下载、移动、重命名、新建和删除其中的文件。构建与运行环境按这个目标校验。

它采用“一体化二进制”部署：Axum 程序既处理 API，也返回构建时嵌入的 xcss-web WASM、浏览器绑定和 xcss CSS。浏览器 Rust 实现负责登录、页面和业务状态，认证合同继续复用 xcss。生产环境通常在它前面放置 HTTPS 网关，负责 TLS 和公网边界。

## 1.2 用户看到什么

用户从浏览器看到的核心流程是：

1. 打开 HTTPS 地址；
2. 输入 xcss 管理员 username 和密码；登录 candidate 由共享规则执行 ASCII trim/lowercase 后变成 canonical username；
3. 进入共享根，面包屑最左侧的房子图标代表根目录；
4. 分页浏览或搜索文件；
5. 在一列 Actions 中通过“移动、下载、删除、重命名”四个固定槽操作项目；
6. 选择文件或文件夹上传；
7. preflight 已发现可替换目标时先做批量确认；若提交时才出现晚到重名，再对该文件确认；
8. 页面根据操作结果刷新或把列表标记为可能过期。

当前版本只下载单个文件，不提供目录 ZIP。目录仍可以通过文件夹选择器上传；“文件夹上传”实际是把所选文件的相对路径逐个上传，空目录不会凭空出现。

## 1.3 适用场景

当前面向单节点、站点根路径部署和互信管理员。每位管理员都拥有共享根完整权限；页面、API 和静态资源由同一 Rust 程序提供。上传按文件处理，下载面向单个文件；页面刷新后的继续上传需要重新选择本地文件并核对原任务。

这些选择使源码围绕浏览器文件管理展开。各能力与相邻功能的详细取舍见[设计矩阵](../feature-decision-matrix.md)。

## 1.4 四层心智模型

可以把项目分成四层：

```mermaid
flowchart TB
    P[展示层<br/>xcss-web / WASM / HTML / CSS]
    H[HTTP 层<br/>连接、路由、认证、响应]
    D[领域协调层<br/>列表、上传、移动、删除]
    I[基础设施层<br/>RootedFs、SQLite、日志、任务与磁盘]
    P --> H
    H --> D
    D --> I
```

### 展示层

位于 [web](../../web)。它负责页面、键盘交互、对话框、列表 DOM、上传队列以及把后端问题显示给用户。

### HTTP 层

入口主要位于 [src/main.rs](../../src/main.rs) 和 [src/server/router.rs](../../src/server/router.rs)。它负责 TCP、HTTP/1.1、路由分类、认证、CSRF、请求大小和超时边界。

### 领域协调层

位于 `src/server/` 下的 listing、upload、download、browser_api、delete 等模块。它把一次用户操作拆成“验证、预留、执行、提交、记录结果”。

### 基础设施层

包括安全的根目录相对文件访问、SQLite 状态 actor、磁盘空间预算、任务跟踪、日志和清理 worker。这一层决定项目在并发、超时和崩溃时是否仍可信。

## 1.5 两个持久化区域

Xczs 同时使用两个目录，不能混在一起：

| 区域 | 保存内容 | 用户是否直接看到 | 备份意义 |
| --- | --- | --- | --- |
| 共享根 `serve-path` | 用户的文件和目录、目标父目录私有子目录中的上传 stage、删除 trash | 普通业务文件可见；内部保留项隐藏 | 这是业务数据主体 |
| 状态目录 `data_dir` | `state.sqlite3` 及其 journal | 不可通过网页访问 | 保存操作、上传、清理的控制状态 |

状态目录必须在共享根之外，不能是其子目录，也不能让共享根成为状态目录的子目录。程序还会校验别名和底层身份，避免通过符号链接等方式把两者偷偷指向同一位置。

## 1.6 为什么同时需要文件系统和 SQLite

只写文件系统不够，因为进程崩溃后需要回答：

- 某个 Operation ID 是否执行过？
- 上传已经持久化到哪个偏移？
- 一个完整 stage 是否正在等待覆盖确认？
- 删除目标已经移入隐藏回收区，但是否还没清理完？

只写 SQLite 也不够，因为真实文件内容、权限、所有权、扩展属性和目录结构仍在 Linux 文件系统中。

因此项目采用“文件系统保存数据，SQLite 保存协议状态”的分工。两者无法组成一个跨系统事务，所以代码必须显式表达提交点和不确定状态。第 5、7 章会详细解释。

## 1.7 一次“重命名”的完整层次

以浏览器点击“重命名”为例：

```mermaid
sequenceDiagram
    participant U as 用户
    participant J as workspace.rs / operations.rs
    participant R as Router
    participant A as browser_api.rs
    participant S as StateStore
    participant F as RootedFs
    U->>J: 原文件名处进入编辑
    J->>R: POST /__xczs__/api/rename
    R->>R: 会话、来源、CSRF、正文限制
    R->>A: 已验证的请求上下文
    A->>S: 预留 Operation ID
    A->>F: 在同一父目录执行安全重命名
    A->>S: 写入最终操作状态
    A-->>J: 成功 / 已知失败或冲突 / 未知
    J->>J: 成功时刷新；已知未提交时保留；未知时标记待刷新
```

这张图最重要的不是函数名，而是三个不同事实：

- 前端编辑框只是收集新名字；
- HTTP 成功或错误是协议结果；
- 磁盘对象是否真的改名由提交边界决定。

## 1.8 仓库目录地图

```text
xczs/
├── web/                 浏览器 Rust、WASM 绑定和样式
│   └── xcss-web/src/               列表、操作、上传、API 等 Rust 模块
├── config/                       唯一当前源码配置样例
├── systemd/                      普通账号服务单元
├── libexec/                      安装器内部实现
├── config/xssi.lock             固定独立安装维护仓库提交
├── docs/                   设计、取舍、运维和本教学手册
├── scripts/                质量门、文档检查、发布打包脚本
├── src/                          Rust 库和可执行程序
│   └── server/                   HTTP 业务、文件系统和状态模块
├── tests/                        Rust 集成测试和浏览器端到端测试
├── .node-version                 当前 Node 26.7.0 仓库级版本基准
├── build.rs                      精确 AMD64 GNU 编译守卫与 Git SHA 注入
├── Cargo.toml                    Rust 包、依赖和 release 配置
├── package.json                  前端检查与测试命令
└── rust-toolchain.toml           固定 Rust 1.99 工具链
```

源码配置一律位于 `config/`，部署资产一律位于 `systemd/`，客户端一律位于 `web/`。生产安装故意使用 `ROOT/config/config.json` ：这是 Linux FHS/证书权限边界，不是仓库目录命名不一致，也不是历史兼容路径。

### `src/` 的重点

- [main.rs](../../src/main.rs)：进程入口、监听器、连接配额和优雅停机；
- [args.rs](../../src/args.rs)：CLI/JSON 配置、默认值和启动前验证；
- [auth.rs](../../src/auth.rs)：密码哈希与认证相关基础逻辑；
- [server.rs](../../src/server.rs)：`ServerBuilder` 和运行时依赖组装；
- [server/router](../../src/server/router.rs)：请求画像与路由分发；
- [server/browser_api.rs](../../src/server/browser_api.rs)：新建、移动、重命名、上传预检等浏览器 API；
- [server/listing.rs](../../src/server/listing.rs)：目录列表、搜索、分页快照；
- [server/download.rs](../../src/server/download.rs)：单文件和单段 Range 下载；
- [server/upload.rs](../../src/server/upload.rs)：上传 façade、共享事务类型和阶段装配；
- [server/upload/prepare.rs](../../src/server/upload/prepare.rs)、[target.rs](../../src/server/upload/target.rs)、[transfer.rs](../../src/server/upload/transfer.rs)、[commit.rs](../../src/server/upload/commit.rs) 与 [failure.rs](../../src/server/upload/failure.rs)：上传准备、目标 identity/revision、正文传输、原子提交和失败收口；
- [server/maintenance.rs](../../src/server/maintenance.rs)：上传 stage、删除 trash 与持久化清理任务的统一后台维护；
- [server/disk_space.rs](../../src/server/disk_space.rs)：按文件系统设备分桶的空间预留与 readiness 空间检查；
- [server/blocking_io.rs](../../src/server/blocking_io.rs)：为可能卡住的文件系统调用提供有界 blocking admission；
- [server/delete.rs](../../src/server/delete.rs)：持久化删除的协调；
- [server/rooted_fs.rs](../../src/server/rooted_fs.rs)：锚定共享根的安全文件系统操作；
- [server/state_store.rs](../../src/server/state_store.rs)：SQLite actor 的外部接口。

### `web/xcss-web/src/` 的重点

- [lib.rs](../../web/xcss-web/src/lib.rs)：启动、会话与登录；
- [http.rs](../../web/xcss-web/src/http.rs)：Fetch/XHR、超时、预算和 Operation/job 查询；
- [model.rs](../../web/xcss-web/src/model.rs)：工作区、列表、筛选和上传数据；
- [workspace.rs](../../web/xcss-web/src/workspace.rs)：列表、分页、搜索、行内编辑和焦点；
- [operations.rs](../../web/xcss-web/src/operations.rs)：新建、移动、重命名、删除和确认；
- [protocol.rs](../../web/xcss-web/src/protocol.rs)：严格上传响应矩阵及绑定验证；
- [upload.rs](../../web/xcss-web/src/upload.rs)：预检、选择、队列、正文、恢复、取消与终态历史；
- [tags.rs](../../web/xcss-web/src/tags.rs)：标签、关联、筛选、索引与扫描。

## 1.9 主要技术为什么合适

### Rust

适合构建单文件部署、低运行时开销的服务，并能用类型和所有权约束共享状态。代价是文件系统并发和异步错误路径的代码比脚本语言更显式。

### Tokio + Axum

Tokio 提供异步任务、网络、信号和同步原语；Axum 组织路由及请求/响应适配，xcss Runtime 在底层管理 Hyper HTTP/1.1 连接、正文流、超时和停机。

### Rust / xcss-web 与浏览器 WASM

xcss-web CSR 在浏览器内渲染登录、导航、列表、标签和上传界面，使用信号和生命周期清理保存状态及释放资源。Vite 收集 wasm-bindgen 绑定、WASM、CSS、字体和许可；生产不需要 Node 服务。主要代价是增加 WASM 编译工具链、调试与 JavaScript 互操作成本，收益是否体现为体积或速度需要实际测量。

### SQLite

状态量小、与单进程服务一起部署，SQLite 能提供事务和崩溃恢复而不引入外部数据库。专用 actor 线程让连接所有权和命令顺序清晰。

### Linux `openat2`

共享根是安全边界，普通字符串拼接和 `canonicalize` 难以抵抗并发路径替换。锚定根目录 FD 并使用 Linux 的受约束路径解析更适合这里的威胁模型；代价是服务端只能运行在支持该能力的 Linux 上。

## 1.10 三种并发不要混淆

项目中至少有三种并发：

1. **网络并发**：多个 TCP 连接和 HTTP 请求同时到达；
2. **业务并发**：两个用户同时操作同一路径，或列表期间目录发生变化；
3. **阻塞 I/O 并发**：文件系统和 SQLite 操作可能卡住，不能长期占用 Tokio 异步执行线程。

连接信号量、路径协调器、Operation Registry、阻塞任务跟踪器解决的是不同问题，不能因为它们都叫“限制并发”就合并成一个锁。

## 1.11 新手最容易犯的五个误解

1. **把 URL 路径直接当磁盘路径。** URL 需要先规范解码和验证，磁盘操作必须从根 FD 相对执行。
2. **认为收到超时就一定没执行。** 普通写操作可能已经分离提交；上传只有在服务端总 deadline 原子关闭首次 mutation boundary 时才能证明本次 `not-started`，task 先越界或客户端自身超时都不能这样推断。普通写操作要用原 Operation ID 查询，上传要用原 Upload ID 对账。
3. **认为列表结果永远实时。** 分页使用短寿命快照；写操作会使旧快照失效。
4. **认为覆盖就是直接打开目标并截断。** 上传先写目标父目录下私有子目录中的 stage，再在校验后原子发布。
5. **认为 SQLite 记录成功就等于磁盘成功。** 两个持久化域必须按规定顺序同步，失败时有时只能报告未知。

## 1.12 本章动手检查

在仓库根目录执行：

```sh
rg --files src/server web/xcss-web/src tests/frontend | sort
rg "MKDIR_API_PATH|MOVE_API_PATH|RENAME_API_PATH" src
rg "struct .*Error|enum .*State" web/xcss-web/src
```

尝试回答：

1. 浏览器启动、工作区、HTTP 和上传状态分别由哪个 Rust 模块负责？
2. 为什么状态目录不能放在共享根里面？
3. 如果删除请求显示“请求超时”，你会先用原 Operation ID 查询，还是立即生成新 ID 再删一次？为什么？

下一章会真正准备环境并启动服务。
