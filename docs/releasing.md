# 构建输入与发行验证

xcss 是编译期供应链输入，不是运行时共享服务。当前 Rust 固定正式 1.0.5 / `6a8113d4be76e5e8977ac7be08c1a7c57963ae49`，一个 @xcss/web 包使用同版 GitHub Release tarball 和锁文件 integrity，无相邻工作区依赖。Xczs 1.0.3 的独立源码树构建、浏览器回归和发行核验由当前提交的 CI 与标签工作流执行。后续发行仍须执行同样门禁；依赖不可取得或身份不符时停止，不能复制共享类型、目标守卫或认证实现继续构建。

`xcss-web-build.json` 声明根 Web、`build:platform`、`web/runtime-dist` 和 Xczs Cargo package。
正式源码构建通过 `npm run build:server:release` 完成前端→Rust→实际 binary 资源验收。`build:platform`
把类型检查输出保留在 `web/dist`，并将实际 native/React 资源组装至 runtime-dist；所有清单、摘要、
内嵌字节与 HTTP 资源响应由 xcss 共同实现。完整清单摘要构成正式资源 URL 的命名空间，
固定文件名也可以安全使用 immutable 缓存；动态 HTML 和用户文件继续使用原来的私有缓存规则。

开发通过 `npm run build:server` 明确生成 `unbound` binary。选择 `--development` 并设置绝对路径
`XCZS_DEV_WEB_DIR` 后，当前目录资源采用共同 `no-cache`/ETag 服务，后续前端构建不要求重编 Rust。
该 provider 不依据编译时文件名单限制新 chunk，并逐次进行 no-follow/普通单链接文件验证。正式或
非开发模式拒绝覆盖，且拒绝发生在任何管理员状态创建与业务数据库写入之前。

资源输出的清理范围通过 xcss `assertWebOutput` 校验，不允许指向源码、仓库祖先、Git metadata、
另一个项目或链接目录。专用外部 stage 与默认 runtime-dist 允许重建；发行清单始终绑定 binary。

## CI 与发行通道

仓库的 `.github/workflows/read-only-ci.yml` 只提供远程回归反馈：权限为 `contents: read`，checkout 不保留凭据，静态、Rust、质量和 Chromium/Firefox 层不会创建 tag/release 或签名，也不会上传制品。质量层分别运行覆盖率、部署行为、发布脚本自测和 release binary smoke；主分支推送总是运行，普通 PR 保持轻量，修改工作流、依赖清单、部署样例或发布/部署脚本的 PR 则在合并前运行完整质量层。各步骤只在自己的前置条件成功时运行，一项实质检查失败不会跳过其余独立检查。唯一当前 Node 26.7.0 由 `.node-version`、manifest/lockfile 和工作流共同声明；`scripts/check.sh` 与正式打包入口还会在任何审计、构建或依赖代码前精确比对实际运行时，因此 npm 的 `EBADENGINE` warning 不能形成绿色结论。Rust 1.99.0、ShellCheck 0.11.0、锁定的 npm 工具和 Action commit SHA 也在工作流中固定；静态、Rust 与浏览器 job 使用 `ubuntu-24.04`，部署质量 job 使用 x64 `ubuntu-26.04`，两种托管镜像的实际版本及宿主工具均写入日志。GitHub 当前把 26.04 标为 preview；若该 runner 不可调度或镜像回归，质量门必须保持失败。合并前应查看全部矩阵结果，但它不包含正式签名边界，也不替代目标 exact tag 上的完整本地门和下述发布流程。

`.github/workflows/release-binary.yml` 只在推送 `v<version>` 注释标签后运行。`preflight` 校验标签、Cargo 版本与完整源码 SHA；只读 `verify_build` 执行锁定依赖审计、生成 Web 资源并直接编译 Linux release 二进制，复核版本、动态库、SHA-256 与发行输入摘要。唯一的 `contents: write` job 不 checkout、不执行下载的二进制，只消费本次只读构建上传的精确 artifact 并复核后发布。完整签名包 E2E 可按需手动执行，不阻塞便捷二进制 Release；主分支 CI 继续承担测试、浏览器、覆盖率和部署回归。

`.github/workflows/formal-release-e2e.yml` 可人工触发，使用 `ubuntu-26.04` 和临时 Ed25519 密钥调用完整的 `scripts/package-release.sh`。它在隔离 clone 中运行完整质量门、vendor、release build、SBOM、checksum、签名和原子目录发布，并独立复核四项制品、签名、公钥、包内 `SHA256SUMS` 及完整版本/SHA。该任务只有只读权限，不使用生产签名密钥；其临时密钥和验收产物不构成便捷 Release 的生产信任根。

自动 GitHub Release 是面向直接下载运行的便捷通道：其二进制在 `ubuntu-24.04` 托管 runner 上构建，必须匹配目标 CPU、glibc、动态加载器与 `openat2` 内核能力。需要 CycloneDX SBOM、第三方和标准库许可证清单、构建环境记录、可重复归档及独立公钥签名时，仍必须执行下述本地正式发布流程；不能把同一 Release 中的 checksum 当作独立信任根。

## 正式发布输入

发布包由 `scripts/package-release.sh` 从干净 Git 提交构建。`Cargo.toml` 的版本必须存在精确的 `v<version>` tag，且该 tag 必须指向当前 HEAD。脚本强制执行完整 `scripts/check-release.sh`，不是依赖调用者事先声称检查通过；门禁后、签名前和发布前都会再次核对 HEAD、tag、版本与干净状态。运行包只携带二进制、配置/网关样例、运行说明、许可证、SBOM、依赖清单、构建记录和校验文件；源码、测试和完整教程由 `source_sha` 指向的仓库提交提供。

进入构建前，Git 索引和目标 commit tree 中的条目必须都是 mode `100644/100755` 的普通 blob；tracked symlink（`120000`）、submodule/gitlink（`160000`）及其他类型一律拒绝。权限为 `0700` 的私有 bare façade 只含由脚本摘要锁定的最小 local config，并通过 object alternates 读取目标对象库；所有决定源码身份的 Git 命令都清空 HOME、system/global 配置并禁用额外 attributes/replace。

脚本从 façade 解析一次完整 commit ID。它先生成并验证一份质量门 archive，在没有 `.git` 的 `0700` 私有副本中运行检查；门禁结束后用独立 snapshot index 比较 tracked 内容和 mode，并拒绝任何非忽略新增路径。随后整棵质量树及其缓存被删除，再从同一 commit 分别生成全新的签名构建归档和打包归档。每份 tar 都作为独立文件保存到私有 stage 并立即验证，再解包并用目标 commit tree 建立独立临时 index；解包树会以 no-follow 方式拒绝 symlink 及任何非普通文件/目录条目，缺失、额外、类型、mode 或内容不同都会失败。后两份 tar 的 SHA-256 还必须完全相同。因此本地 replace object、private attributes、质量工具或构建期间改变 worktree/Git 元数据，不能让同一声明 SHA 对应另一棵检查、构建或打包树。只有最后一份重新验证的树提供文档和部署材料。同 UID 恶意进程仍属于必须用身份/主机隔离解决的边界。

## 隔离构建与依赖审计

隔离质量门以 `env -i` 启动，固定 PATH、Rust 工具链和完整源码 SHA，并使用私有 HOME、Cargo home/target、npm cache、XDG 目录与临时目录。Cargo 先从锁文件 vendor，再以 offline source replacement 运行；这与之后签名构建使用的独立 vendor 树相互隔离。npm cache 播种器只接受 `package-lock.json` 中带 HTTPS resolved URL 与 SHA-512 integrity 的条目，并重新散列宿主 cache 内容后写入私有 cache；`npm ci` 使用 `prefer-offline`，缺失包以及 `npm audit` 仍可能访问网络。宿主 RustSec Git 数据库只有在 canonical origin、`HEAD=FETCH_HEAD`、实体 `FETCH_HEAD` 时间戳不得比当前时间早超过 7 天或晚超过 300 秒，并通过完整物理/Git/内容检查后才可复用；alternates、不安全元数据、symlink/submodule/特殊项、untracked 路径和 tracked 内容/mode 漂移均拒绝。合格输入以无硬链接私有 clone 封存 revision、fetch epoch、index/config 校验和；不合格、过期或缺失时，在任何项目或依赖代码前用 dummy lockfile 在私有数据库联网刷新，离线失败关闭。发布入口先执行 `cargo audit --db ... --no-fetch --no-yanked` sealed pre-audit；随后用私有 Cargo home 执行 `cargo fetch --locked`，保证 yanked 检查拥有完整锁图所需的 crates.io 索引项，再以同一封存数据库运行 `cargo audit --no-fetch --deny yanked`。索引缺失、抓取失败或锁图含已撤回 crate 都失败关闭；该 Cargo home 每次全新创建，因此当前正式发布要求 registry 网络可达，宿主 Cargo 缓存不能替代这一步。之后通过必填 `XCZS_QUALITY_AUDIT_DB` 把同一数据库交给隔离 `scripts/check.sh`，该脚本也在其他项目/依赖步骤前先审计。封存时校验 seal 与新鲜度，pre-audit 和 yanked 检查后重验 seal；完整门禁后重验 seal 与新鲜度，随后销毁质量树和该 RustSec 数据库。包内环境清单只记录 advisory revision/fetch epoch，不记录内部 seal 摘要。Playwright 只复用显式浏览器 cache，不让测试依赖用户 npm/Cargo 配置。JavaScript 安全门固定使用 ESLint 核心规则与 Mozilla `no-unsanitized` 规则，检查语法、基础格式、常见动态 HTML/动态执行/原生模态 API，并限制 `fetch` 与 XHR 的所属模块；策略单测验证常见拒绝和合法边界。TypeScript 7.0.2 另以 `strict + noEmit + isolatedModules + verbatimModuleSyntax` 检查全部生产 TypeScript/TSX，外部/解析输入保持为 `unknown` 并经守卫收窄，生产源码不保留显式或隐式 `any`。两者都不替代运行时守卫或完整跨过程污点证明。本地有 ShellCheck 时统一门执行 warning 检查，缺失时明确跳过且不联网安装；远程 CI 固定并强制执行 0.11.0。

脚本严格校验 Rust/rustc/Cargo 1.99.0、`cargo-cyclonedx 0.5.9` 与 `cargo-audit 0.22.2`。固定工具链 sysroot 的 `share/doc/rust/COPYRIGHT-library.html` 必须是 sysroot 内 no-follow 普通文件，并精确匹配发布脚本中对 Rust 1.99.0 固定的已审核 SHA-256；未知工具链没有审核摘要时直接拒绝。验证后的副本以 `RUST-STANDARD-LIBRARY-COPYRIGHT.html` 打包。签名构建另用锁文件 vendor 依赖，随后以清空环境、私有 Cargo home、离线 source replacement、关闭增量编译和显式编译器运行 release 构建；完整 Git SHA 嵌入版本字符串，私有构建路径经过 remap 并在二进制中复查。`SOURCE_DATE_EPOCH` 同时传给 Rust 构建、SBOM 和归档；未显式设置时使用提交时间。

## SBOM 与许可证

SBOM 递归把本地 Xczs `bom-ref`/`purl` 规范化为绑定完整源码 SHA 的稳定 Cargo 标识；source revision 只接受恰为 40 或 64 位的小写十六进制对象 ID，并拒绝明文或百分号解码后出现的本地 `file:`、POSIX/Windows 绝对路径与构建根。它要求元数据中恰有一个本地 Xczs root 和一个依赖 root；这是项目所需的结构/无路径泄漏检查，不替代完整 CycloneDX schema validator。

`THIRD_PARTY_LICENSES.txt` 从 Cargo metadata 中 Xczs 可达的非开发依赖生成，依赖源码必须位于本轮 vendor 根。每个包必须声明非空、经审核的 SPDX `license` 表达式；metadata `license_file` 只用于收集上游正文，不能替代表达式或作为分类 fallback。生成器按 `WITH > AND > OR` 优先级解析真实 SPDX AST，只接受审核清单内的 license identifier/exception，并要求表达式存在一条完整 permissive 选择：`OR` 任一分支可行，`AND` 两侧都必须 permissive；只对明确列出的 Cargo 遗留 `MIT/Apache-2.0` 和 `Unlicense/MIT` 写法映射为 `OR`。例如 `LGPL AND (MIT OR Apache-2.0)` 会拒绝，而 `(LGPL AND Apache-2.0) OR MIT` 可选择完整 MIT 分支。

生成器收集 metadata `license_file` 与包根下 LICENSE/COPYING/NOTICE 的常规文件。每个候选必须是依赖源码及 vendor real root 内的 no-follow 普通文件；所有当前 xcss crate 必须携带真实 Apache-2.0 文本，字体必须携带 OFL。缺失任何必要许可证即失败，不能用产品许可证或按名称特判绕过。

包内 `BUILD-ENVIRONMENT.txt`、SBOM、第三方 notice、Rust 标准库 notice 和项目 Apache-2.0 许可证均纳入 `SHA256SUMS`。

当前完整运行包由 `install_release_support_tree` 复制 `bin/xczs`、`config/`、`systemd/`、`libexec/`、`install.sh`、`LICENSE-APACHE` 和 `docs/runtime-package.md`，后者在包内重命名为 `README.md`。源码、测试和完整文档通过 `BUILD-ENVIRONMENT.txt` 中的 `source_sha` 定位。打包检查核对最终运行包的必要文件、模式与校验清单。

## 输出与签名

输出目录必须由当前发布账号拥有且不能让 group/other 写入；它会被解析为物理路径并通过已验证 fd 持有独占 `flock`。stage 创建、构建、清理、最终 rename 和目录同步均从锁定的目录 fd 路径派生，公开字符串路径在此后只用于身份复核和结果展示，祖先目录换绑不能重定向 mutation。发布后还会核对公开路径、锁定目录和最终 release 的 dev/inode；若公开路径被换绑则报告失败，但不会回滚已经完整提交到锁定目录的制品。

签名 key 参数在构建阶段只作为尚未解析的调用输入保存。Cargo、rustc、依赖构建脚本、Node、SBOM、第三方与标准库 notice、最后一份源码验证、包内文档检查、归档和 checksum 全部完成，并再次通过 exact-source gate 后，脚本才进入短生命周期签名子进程：在其中解析并要求私钥是当前账号拥有、mode `0400`/`0600`、单硬链接的普通文件，打开 fd 后复核 dev/inode，完成签名和验签，随后由进程退出关闭 fd。密钥算法还必须属于明确的发布 allowlist：Ed25519、Ed448、至少 3072 bit 的 RSA，或曲线为 `prime256v1`、`secp384r1`、`secp521r1` 的 ECDSA。弱 RSA、DSA、`secp256k1` 等未审核曲线、X25519 等非签名密钥，以及无法确定类型/强度的 key 都在签名前失败关闭。构建工具不会继承私钥 fd。但这不是同 UID 恶意代码隔离；正式签名应在独立账号、隔离主机或 HSM 中执行。

发布输出文件系统必须支持 Linux `RENAME_NOREPLACE`。脚本使用 GNU `mv --update=none --no-copy`，并且只有 source 消失、destination 是实体目录且设备号/inode 与移动前 source 相同时才确认发布；静默碰撞或身份不符都会失败。

脚本在同一文件系统的 `0700` 私有 stage 中构造一个完整的 `<release-name>.release` 目录。归档、外层 checksum、签名和公钥全部写完、验签并同步后，只用一次 no-clobber 目录 rename 公开该目录，再同步输出目录；rename 与输出目录同步组成一个暂时忽略 HUP/INT/TERM 的短提交段，普通信号不会卡在两者之间。公开名称因此不会经历“只有部分 sidecar”的状态，也不会覆盖已有文件、目录或 symlink。若在 rename 前遭遇 SIGKILL 或掉电，可能留下不可见的 `.xczs-release-stage.*` 私有目录；若在 rename 后、输出目录同步确认前遭遇 SIGKILL、掉电或同步错误，公开目录仍是一次 rename 产生的完整目录，但该目录项跨重启的持久性尚未确认，必须把该次发布视为失败并人工核验，不能仅凭“已经可见”判定发布成功：

```sh
cargo install cargo-cyclonedx --version 0.5.9 --locked
cargo install cargo-audit --version 0.22.2 --locked
chmod 0600 /secure/offline/xczs-release-key.pem
install -d -m 0700 ./dist
./scripts/package-release.sh \
  --signing-key /secure/offline/xczs-release-key.pem \
  --output-dir ./dist
```

固定时间戳、权限、经 commit-tree 复核的源码快照、离线 Cargo vendoring、隔离 Git/Cargo/npm 质量环境、编译路径映射、审核过的标准库 notice 摘要和 SBOM 根引用消除了脚本已知的路径、时间、replace object、private attributes 和用户 Git/Cargo/npm 配置非确定性。逐字节可重复归档仍要求相同源码、`SOURCE_DATE_EPOCH`、Rust/Node/npm/cargo-cyclonedx/coreutils/OpenSSL 工具版本、host target 及相同的已验证依赖内容；应在不同长度的 checkout 路径各构建一次并比较归档 SHA-256。外层签名是否逐字节相同还取决于所用密钥算法；验收依据是签名能验证同一归档 checksum，而不是签名字节相等。

## 验证收到的签名包

生产主机验证时，公钥必须通过独立可信渠道取得并固定，不能只相信与归档从同一位置下载的临时公钥：

```sh
set -eu

bundle=/secure/releases/xczs-1.0.3-x86_64-unknown-linux-gnu-0123456789ab.release
pinned_public_key=/secure/trust/xczs-release-public.pem
test -d "$bundle"
test ! -L "$bundle"
test -f "$pinned_public_key"
test ! -L "$pinned_public_key"
cd -- "$bundle"
release_name="${bundle##*/}"
release_name="${release_name%.release}"
archive="${release_name}.tar.gz"
checksum="${archive}.sha256"
signature="${checksum}.sig"
openssl dgst -sha256 -verify "$pinned_public_key" \
  -signature "$signature" "$checksum"
sha256sum --check "$checksum"

verify_root="$(mktemp -d)"
chmod 0700 "$verify_root"
tar --extract --gzip --file "$archive" --directory "$verify_root"
release_dir="$verify_root/$release_name"
test -d "$release_dir"
test ! -L "$release_dir"
(cd "$release_dir" && sha256sum --check SHA256SUMS)

# 从独立可信的发布记录填写完整值，不从同一下载目录自行推断。
expected_version=1.0.3
expected_sha=0123456789abcdef0123456789abcdef01234567
expected_target=x86_64-unknown-linux-gnu
expected_common_revision=6a8113d4be76e5e8977ac7be08c1a7c57963ae49
test "$("$release_dir/xczs" --version)" = \
  "xczs $expected_version (git $expected_sha) xcss=$expected_common_revision"
grep -Fx "format=xczs-build-environment-v1" \
  "$release_dir/BUILD-ENVIRONMENT.txt"
grep -Fx "source_sha=$expected_sha" "$release_dir/BUILD-ENVIRONMENT.txt"
grep -Fx "source_version=$expected_version" \
  "$release_dir/BUILD-ENVIRONMENT.txt"
grep -Fx "target=$expected_target" "$release_dir/BUILD-ENVIRONMENT.txt"
grep -Eq '^source_date_epoch=[0-9]+$' "$release_dir/BUILD-ENVIRONMENT.txt"
for key in \
  bash rustc cargo cargo_cyclonedx cargo_audit \
  rustsec_advisory_db_revision rustsec_advisory_db_fetch_epoch \
  node npm git openssl tar gzip mv sha256sum
do
  grep -Eq "^${key}=.+$" "$release_dir/BUILD-ENVIRONMENT.txt"
done
# 验证完成后，只删除本次 mktemp 返回的精确目录。
# rm -rf -- "$verify_root"
```

上面的 `openssl dgst` 适用于发布策略允许的 RSA（至少 3072 bit）和 ECDSA（`prime256v1`、`secp384r1`、`secp521r1`）digest 签名密钥。若固定公钥是 Ed25519 或 Ed448，发布脚本会改用 EdDSA 原始消息模式，验签命令应替换为：

```sh
openssl pkeyutl -verify -rawin -pubin \
  -inkey "$pinned_public_key" \
  -sigfile "$signature" \
  -in "$checksum"
```
