# 02. 环境准备与第一次运行

本章先跑通一个隔离的开发实例，再解释每个参数的作用。不要一开始就把真实照片、工作文档或生产目录交给正在学习的实例。

## 2.1 平台要求

构建源码需要：

- 唯一目标平台是 Linux AMD64 GNU，即 `x86_64-unknown-linux-gnu`；aarch64、ARM64、musl、Windows 和 macOS 均在编译期拒绝；
- Rust、rustc、Cargo 1.99.0；
- 源码使用 Rust 2024 edition；
- 与目标 ABI 匹配的 C 编译器、链接器和 binutils；项目通过 `SQLx` 的 sqlite-bundled feature 编译 SQLite C 源码。

运行已经构建好的二进制不需要 Rust 工具链，但制品必须匹配目标机器的 CPU 架构和 libc/动态加载器 ABI，内核还必须提供可用的 `openat2`。因此不能把某个 GNU libc 的 x86-64 制品笼统视为“所有 64 位 Linux 都能运行”。

[build.rs](../../build.rs) 会在编译期拒绝任何非 `x86_64-unknown-linux-gnu` 目标。启动时还会探测 `openat2`；精确 target 能编译仍不代表当前内核具备运行所需系统调用。

Node.js 精确版本由根目录 [.node-version](../../.node-version) 唯一声明，用于 JavaScript 类型检查、单元测试、文档检查、Playwright 和发布辅助脚本。工作流通过 `node-version-file` 读取它，本地检查和打包入口也会在运行依赖代码前核对实际 `node --version`。`package.json` 的 engine 表达支持范围，不复制精确版本。生产服务器运行已构建的 Xczs 二进制时不需要 Node.js。

## 2.2 先检查工具

```sh
rustc --version
cargo --version
git --version
cc --version
ld --version
ar --version
node --version
npm --version
curl --version
rg --version
ss --version
```

这里的 `node --version` 必须输出 `v26.7.0`。诸如 `v24.8.1`、`v26.x` 或发行版自行回补的其他版本都不是当前工具链；项目不提供旧/新 Node 的兼容分支，也不会把 npm 的 `EBADENGINE` 警告当作有效门禁。

`cc`、`ld` 和 `ar` 分别代表 C 编译器及链接/binutils 工具；SQLite bundled 源码构建需要它们。后续示例还假定存在 curl、ripgrep 的 `rg`、iproute2 的 `ss`，以及 GNU coreutils 提供的 `mktemp`、`install`、`stat` 和 `rm --one-file-system`。只编译后端时 Node/npm 可以暂缺，但浏览器测试和前端门禁需要它们。

仓库的 [rust-toolchain.toml](../../rust-toolchain.toml) 会让 rustup 自动选择固定工具链。若缺少工具链，rustup 可能需要联网下载。

首次 Cargo 构建可能要从 crates.io 下载未缓存依赖；`npm ci` 可能访问 npm registry，Playwright 安装命令还会下载浏览器。离线环境需要事先准备对应 toolchain、依赖缓存或 vendored 依赖和浏览器制品。依赖审计的联网条件见[第 8 章](08-testing-debugging-and-change-workflow.md#810-日常集成与发行检查)。

前端依赖按锁文件安装：

```sh
npm ci
```

正式前端随 binary 内嵌。首次构建必须安装依赖并生成资源；不能直接把仓库中未经构建的源码当作运行资源。

## 2.3 编译

开发构建：

```sh
npm run build:server -- --no-install
```

输出位于：

```text
target/x86_64-unknown-linux-gnu/debug/xczs
```

发布构建：

```sh
npm run build:server:release -- --no-install
```

输出位于：

```text
target/x86_64-unknown-linux-gnu/release/xczs
```

共同入口依次构建前端、运行 `cargo build --locked`、执行实际 binary 的 `web-assets` 命令，逐个验证
内嵌资源与本次输出相同。正式模式还要求干净源码与完整 Git revision；开发模式把源码身份设为 `unbound`。
`--locked` 要求 Cargo 严格使用 [Cargo.lock](../../Cargo.lock)，避免一次普通构建意外改变依赖解析结果。
当前 xcss Rust/Web 依赖固定正式 1.0.5 的完整 revision、Release URL 和锁文件 integrity，无需相邻 xcss checkout。Xczs 1.0.3 的独立构建与发行验收由当前提交的 CI 与标签工作流执行。后续改动仍须通过发行门禁，不能仅凭本地编译成功宣称已发布，也不能复制认证、Schema 或 target 合同代替共享包。

内嵌资源清单由 xcss 自动生成，覆盖 native 文件控制器与 React/font/icon/许可证资源；
TypeScript 检查使用 `noEmit`；运行资源由 Vite 构建到 `web/runtime-dist`。调用 `xczs web-assets` 可查看程序中的精确清单。

开发热更新使用未绑定的开发 binary。设置 `XCZS_DEV_WEB_DIR` 为 `web/runtime-dist` 的绝对路径，并
显式传入 `--development`。该模式只允许 loopback 监听，资源每次请求重新读取，支持新增 Vite chunk；
重新 `npm run build:platform` 后不必重新编译 Rust。正式 binary 或没有 `--development` 时，目录
覆盖在创建管理员账户、根锁及业务数据库之前拒绝。

## 2.4 准备隔离目录

创建两个互不包含的目录：

```sh
tutorial_root="$(mktemp -d /tmp/xczs-learning.XXXXXXXX)" &&
chmod 0700 "$tutorial_root" &&
mkdir "$tutorial_root/shared" &&
install -d -m 0700 "$tutorial_root/state" &&
printf 'hello from xczs\n' > "$tutorial_root/shared/hello.txt" &&
mkdir "$tutorial_root/shared/examples" &&
printf 'Tutorial root: %s\n' "$tutorial_root"
```

后续命令沿用当前 shell 中的 `$tutorial_root`，不要关闭这个终端。`mktemp -d` 产生不可预测且仅当前用户可进入的目录，避免在多人机器的 `/tmp` 下沿用一个可能已被预置或换成符号链接的固定名称。

这里：

- `shared` 是浏览器能管理的共享根；
- `state` 是私有状态目录；
- `0700` 表示只有当前 Linux 用户可读、写和进入状态目录。

正式部署时，状态目录应由专用服务账号拥有，并放在共享根之外的稳定磁盘位置。`/tmp` 只适合本章实验。

## 2.5 准备初始管理员

新实例通过 `init --username admin` 从标准输入读取密码（单行，可带末尾换行），写入受保护的管理员存储。使用安全密码输入或秘密管理器管道，不把密码放进命令参数、JSON、历史或日志。密码遵循公共管理员策略；配置不需要手工 PHC。

## 2.6 启动最小实例

先复制受保护配置模板，把其中的共享根、状态目录替换为本章生成的真实值，再显式初始化状态库并启动：

```sh
cp config/config.json.example "$tutorial_root/xczs.json"
chmod 0600 "$tutorial_root/xczs.json"
# 用编辑器替换路径，并为本章的小型临时卷设置 min_free_space: 0
target/debug/xczs --config "$tutorial_root/xczs.json" init --username admin
target/debug/xczs --config "$tutorial_root/xczs.json" run
```

`init` 读取并验证这份配置，创建当前状态库及管理员凭据，不启动 HTTP 服务；下一条命令才启动服务。已有当前状态库无需重复初始化，其他版本或身份不匹配的数据库不会被自动迁移。JSON 中的 `min_free_space: 0` 对应 CLI 的 `--min-free-space 0`，只用于很小的临时测试卷，生产应按容量保留余量。

成功后会输出类似：

```text
Listening on http://127.0.0.1:5000/
```

这表示后端已监听，并不表示生产浏览器入口应该使用明文 HTTP。会话 Cookie 带 `Secure`，前端上传 ID 使用安全上下文中的 `crypto.randomUUID()`，因此完整浏览器使用需要 HTTPS 网关。

## 2.7 先用健康接口验证

另开终端：

```sh
curl --noproxy '*' --connect-timeout 2 --max-time 10 \
  -i http://127.0.0.1:5000/healthz
```

预期状态是 `204 No Content`，响应没有正文。该接口公开提供存活检查，只证明进程还能处理 HTTP。

就绪检查也无需登录：

```sh
curl --noproxy '*' --connect-timeout 2 --max-time 10 \
  -i http://127.0.0.1:5000/readyz
```

就绪时返回 `200` 和 `{"ready":true}`，未就绪时返回 `503` 和 `{"ready":false}`。端点读取启动时及每 5 秒更新的探针结果，检查共享根中固定私有探针文件的写入、同步和读回、SQLite 写事务后回滚、可用空间及服务是否正在停止；请求本身不会新建、删除或重命名文件，也不泄露路径和账号。

这里显式使用 `--noproxy '*'`，避免机器上的 `HTTP_PROXY`/`HTTPS_PROXY` 和不完整的 `NO_PROXY` 把回环测试送到外部代理；连接和总时限则避免诊断命令无限等待。

## 2.8 浏览器为什么要经过 HTTPS

推荐拓扑是：

```text
浏览器 --HTTPS--> HTTPS 网关 --HTTP/1.1 回环连接--> Xczs
```

自行管理的 HTTPS 网关负责证书、固定可信域名、规范 Host、上传预算和额外限流。程序不绑定代理软件，不附带网关配置。URI 与业务路径规范由 Xczs 校验。

隔离测试环境由 [tests/frontend/server.mjs](../../tests/frontend/server.mjs) 建立本地 HTTPS 代理，并显式把 `127.0.0.1/32` 配为受信来源。生产必须按真实网关地址设置 `trusted_proxies`，默认空列表。

不要把测试证书直接用于生产，也不要为了省事删除 `Secure` Cookie 或来源校验。

### 最快的本地浏览器体验

如果只是学习界面，可以直接使用仓库的隔离测试服务器：

```sh
npm ci --ignore-scripts --no-audit --no-fund
npm run build:server -- --no-install
XCZS_FRONTEND_TEST_PORT=9443 node tests/frontend/server.mjs
```

这里的网关脚本只使用 Node.js 内置模块；手工浏览不需要下载 Playwright 浏览器。构建嵌入的 Web
资源需要锁定的 npm 依赖。要运行自动化前端测试时，再按[第 8 章](08-testing-debugging-and-change-workflow.md#82-首次准备测试环境)安装 Playwright 浏览器。

然后打开：

```text
https://127.0.0.1:9443/
```

测试登录信息是：

```text
username: frontend-test-0
password: test-password
```

浏览器会因为公开的自签名测试证书显示警告，这是本地测试预期行为。该脚本会创建临时共享根、临时状态目录、多个测试账号和示例文件，在 `Ctrl+C` 后停止后端并清理临时数据。仓库中的测试证书、私钥和固定账号只能用于这个隔离场景，绝不能复制到生产。

在测试服务器仍运行时，可以另开终端直接访问公开就绪端点：

```sh
curl -k --noproxy '*' --connect-timeout 2 --max-time 30 \
  --fail --silent --show-error \
  https://127.0.0.1:9443/readyz
```

`--fail` 让未就绪的 HTTP `503` 产生非零退出状态；就绪时预期正文是 `{"ready":true}`。这里的 `-k` 只用于仓库自签名测试证书，不能作为生产 TLS 校验方式；访问此端点不需要测试账号或 Cookie。

## 2.9 用 JSON 保存配置

第 2.6 节已经复制了配置模板；若需要重新开始，可再次复制并立即收紧权限：

```sh
cp config/config.json.example "$tutorial_root/xczs.json"
chmod 0600 "$tutorial_root/xczs.json"
```

核心结构如下：

```json
{
  "serve_path": "/tmp/xczs-learning.ABCDEFGH/shared",
  "data_dir": "/tmp/xczs-learning.ABCDEFGH/state",
  "bind": "127.0.0.1:5000",
  "min_free_space": 0
}
```

上面的 `ABCDEFGH` 是占位符，两个路径必须改为第 2.4 节的真实绝对路径。JSON 不展开 shell 变量；临时小卷可设 `min_free_space: 0`，仅本机 HTTP 开发时设置 `development: true`。

若第 2.6 节的服务仍在运行，先在原终端按 `Ctrl+C` 正常停止，再回到保有 `$tutorial_root` 的**同一个 shell**启动；否则另一个 shell 没有这个变量，旧实例也仍占用共享根锁和端口。

启动：

```sh
target/debug/xczs --config "$tutorial_root/xczs.json" run
```

JSON 采用严格字段校验。写错字段或保留已经删除的旧配置项会直接失败，不会静默忽略。这样会明确暴露不匹配，并能防止操作者误以为某个安全限制仍然生效。

配置优先级为 CLI > 18 个 `XCZS_*` 环境映射 > JSON > 默认值，每层独立校验。`bind` 为单个含端口 SocketAddr，`serve_path` 必须显式绝对路径。配置文件使用公共只读安全检查：服务账号拥有、单硬链接、`0600` 文件，父目录 `0700`。详细规则见[命令说明](../cli.md)。

## 2.10 常用配置及默认值

当前默认值由 [src/args.rs](../../src/args.rs) 定义。下表使用 JSON 字段名；命令行参数使用 `--` 前缀并将下划线改为连字符，例如 `serve_path` 对应 `--serve-path`。环境映射见[命令说明](../cli.md#环境变量)：

| 配置 | 默认值 | 作用 |
| --- | ---: | --- |
| `serve_path` | 无，必须显式绝对路径 | 必须已经存在且是目录的共享根 |
| `data_dir` | `/var/lib/isarmg/xczs` | 位于共享根之外、权限和属主受校验的持久状态目录 |
| `log_level` | info | UTC 结构化轮转日志级别 |
| `trusted_proxies` | [] | 明确受信代理 IP/CIDR |
| `bind` | `127.0.0.1:5000` | 单个监听 SocketAddr |
| `development` | false | 显式允许 loopback HTTP；所有监听必须为回环地址 |
| `max_upload_size` | 100 GiB | 单文件声明长度上限 |
| `upload_idle_timeout` | 60 秒 | 上传没有进展的允许时间 |
| `upload_total_timeout` | 24 小时 | 单次上传流程总时限 |
| `max_concurrent_uploads` | 4 | 服务端并发上传槽 |
| `min_free_space` | 1 GiB | 计入预留后的最低可用空间 |
| `max_connections` | 256 | 单监听器的连接配额 |
| `max_search_entries` | 10000 | 搜索最多检查的目录项，硬上限 100000 |
| `max_concurrent_searches` | 2 | 并发目录列表和递归搜索数量 |
| `request_timeout` | 300 秒 | 非上传请求在产生响应头前的预算；不限制已经开始的流式下载正文总时长 |

`data_dir` 默认 `/var/lib/isarmg/xczs`；实验必须覆盖为隔离状态目录。数据库文件固定为 `<data_dir>/state.sqlite3`。

## 2.11 启动时发生了什么

从外到内依次是：

1. [main.rs](../../src/main.rs) 构造 CLI；
2. 分派显式 init/run/config/check/status/admin 子命令；
3. [args.rs](../../src/args.rs) 按默认值、JSON、18 个显式环境变量、CLI 的顺序合并并验证配置；
4. 初始化异步日志；
5. `Server::builder(args).build()` 锚定共享根、打开状态库并组装服务；
6. 为每个地址创建 TCP listener；
7. 输出实际监听地址；
8. 接受连接，并在全局信号量许可下交给 Hyper HTTP/1.1；
9. 等待 SIGINT、SIGTERM 或所有 listener 异常退出。

任何启动校验失败都应在接收业务请求前退出，例如：账号为空、哈希格式错误、状态目录不安全、共享根不支持所需能力、监听地址为空。

## 2.12 正确停止

在前台终端按 `Ctrl+C` 会发送 SIGINT。正常排空时 Xczs 会：

1. 停止接受新连接；
2. 给现有连接和任务 30 秒正常完成；
3. 超时后取消普通工作，再给强制收尾 10 秒；
4. 等待已登记的提交排空并关闭状态参与者；
5. 最多等待 5 秒刷新日志；
6. 成功以状态码 `0` 退出，收尾失败以 `1` 退出。

第二个停止信号会提前进入取消普通工作的 10 秒强制收尾阶段；第三个信号或强制阶段超时会结束排空并报告失败。成功和失败路径都会执行有界日志刷新，再显式退出；刷新期间不再另行处理停止信号。失败时不能保证仍在进行的提交或尾部日志已经保存。`kill -9` 会绕过应用停机代码，只应在真正失控时使用。

## 2.13 常见启动失败

### 端口已被占用：`Address already in use`

端口已经被占用。找出监听者：

```sh
ss -ltnp | rg ':5000\b'
```

换端口或正常停止旧进程，不要随意杀死不认识的系统服务。

### 账号参数解析失败

初始化用户名必须符合 xcss canonical 规则：3～64 个 lowercase ASCII 字节，首尾 alnum，字符仅 `[a-z0-9._-]`。密码从 stdin 输入，使用公共密码策略。配置中不填写账户、密码或 PHC。

### 状态目录权限或位置错误

确认它存在、是普通目录、权限私有，并且与共享根没有包含关系：

```sh
stat "$tutorial_root/shared" "$tutorial_root/state"
```

### 同一共享根已有实例

Xczs 会对共享根目录 FD 取得非阻塞独占锁。指向同一根的第二个实例应启动失败；这是防止两个进程各自维护一套内存协调状态。

### `openat2` 不可用

当前内核或文件系统环境不满足安全路径解析要求。不要退回字符串拼接路径，应换到受支持的 Linux 环境。

## 2.14 清理学习环境

先正常停止 Xczs，确认没有仍在使用这些目录的进程，再删除实验目录：

```sh
case "${tutorial_root:-}" in
  /tmp/xczs-learning.[A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9][A-Za-z0-9])
    if [ -d "$tutorial_root" ] &&
       [ ! -L "$tutorial_root" ] &&
       [ "$(stat -c %u -- "$tutorial_root")" = "$(id -u)" ]; then
      rm -r --one-file-system -- "$tutorial_root"
    else
      printf 'Refusing missing, linked, or foreign-owned tutorial path\n' >&2
    fi
    ;;
  *) printf 'Refusing unexpected tutorial path: %s\n' "${tutorial_root:-<unset>}" >&2 ;;
esac
```

不要把这条命令改成指向真实共享根。

## 2.15 本章检查点

你现在应该能解释：

- 为什么共享根和状态目录是两个不同的安全域；
- 为什么 Node.js 是开发依赖而不是生产运行依赖；
- 为什么后端打印 `http://127.0.0.1`，浏览器生产入口仍要求 HTTPS；
- liveness 和 readiness 分别证明什么；
- `--locked` 和 JSON 严格字段校验各自避免了什么意外。

下一章会补齐阅读源码所需的最小 Rust 与 Web 知识。
