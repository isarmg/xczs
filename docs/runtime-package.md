# 当前运行包

当前唯一运行包是 `xczs-VERSION-linux-x86_64.tar.gz`，附带 SHA256SUMS。包内 share/package.json 和 share/package.sig 为 Ed25519 签名清单；签名覆盖全部程序、辅助工具、资源和 systemd 模板。独立信任公钥及 DER SHA-256 指纹不能由下载包自身决定。

ROOT 内含 bin、libexec、share、config、state、logs、run、maintenance；默认 ROOT `/opt/isarmg/xczs`。安装器来自固定 xssi 源码提交。程序、配置、证书、管理员数据库和日志集中在 ROOT；共享文件和实例业务数据可以外置，systemd 文件使用系统目录。

安装入口 install.sh 负责目录、服务账号、权限、发行包校验和 systemd；签名 libexec/setup 负责初始化、当前结构检查、管理员维护和明确支持的数据步骤。server 入口只有 `bin/xczs --root ROOT`，不执行维护或安装，不识别代理或访客 IP。首次安装不自动启动。

自动升级入口 upgrade.sh 下载并验证目标版本，再配合 XSSC 停服、检查、备份、执行已声明数据步骤、切换并启动，最终验证真实版本与 readiness。不重新初始化、不覆盖 live 配置或现有业务文件，不提供旧布局或旧 JSON 管理员兼容。卸载由用户处理。

管理员凭据和会话使用 state.sqlite3 公共表，与其他五个 server 相同；Argon2id 密码哈希、不透明管理员 ID、会话版本和撤销保持一致。状态结构 revision 2，tags.db 继续保留。详细步骤见[首次安装](getting-started.md)和[运维](operations.md)。
