# 部署安全与运维

server 只负责运行，安装与维护属于独立 xssi，自动升级由脚本配合 XSSC 执行。程序不探测或配置 Cloudflare、Caddy，不提取访客 IP，也不解析地址转发头。通用 HTTP 同源、Cookie、CSRF 和业务实例/文件权限继续检查。

固定 ROOT 内包含 bin、libexec、share、config、state、logs、run、maintenance。程序与校验资源由 root 管理，配置目录由 root 控制并允许服务组读取，常规文件为 root:服务组 0640，服务以普通 isarmg-xczs UID 运行。共享文件和实例业务数据可外置；systemd 文件按系统约定放置。

管理员用户名、Argon2id 密码哈希、不透明 ID、会话与认证审计在 state.sqlite3 的公共表中。文件 ACL 引用稳定 administrator_id，改名或改密不会改写文件拥有者。状态线程和管理员池使用 DELETE journal 与 synchronous=EXTRA，停服先排空管理员池再关闭状态线程。

首次安装由脚本SHA-256 校验、检查目录和权限、调用独立辅助程序初始化并生成 systemd，成功后保持停服。自动升级先下载验证新包，再停止全部写入服务、取得维护锁、检查和备份、执行明确支持的数据步骤、替换、启动并核验实际版本与 readiness。恢复使用持久事务，不覆盖启动后新增数据。未知旧结构和旧配置直接拒绝，卸载由用户处理。

真实部署必须验证安装输入失败后重试、现有共享文件保持不变、密码重置撤销会话、升级中断恢复，以及 systemd 实际启动和就绪。具体命令见[运维](../operations.md)。
