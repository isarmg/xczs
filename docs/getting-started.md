# 安装与首次运行

仅接受当前 Linux AMD64 GNU 签名完整包。server 只有 `--root ROOT` 运行入口；安装、初始化和离线维护由独立 xssi 提供，自动升级由升级脚本配合 XSSC 完成。

默认 ROOT `/opt/isarmg/xczs`。配置 `ROOT/config/config.json`，内部状态 `ROOT/state`，日志 `ROOT/logs`，临时文件 `ROOT/run`。外置共享目录必须是已存在、服务账号可访问的目录；安装不得清空文件或递归修改其权限。数据库默认处于 ROOT，也可显式外置实例业务状态。

从 `share/config/config.json.example` 创建受保护输入配置。`serve_path` 指向实际共享目录，`bind` 默认 `127.0.0.1:5000`。程序只使用通用 HTTP、安全 Cookie、同源校验和 CSRF，不识别代理。

```sh
systemd-ask-password 'Initial administrator password:' | sudo ./install.sh \
  --package /absolute/verified-package --root /opt/isarmg/xczs \
  --config /absolute/private-config.json --username admin \
  --public-key /absolute/trusted-public.pem --key-sha256 TRUSTED_DER_SHA256
sudo systemctl enable --now xczs.service
curl --fail http://127.0.0.1:5000/readyz
```

信任公钥及其 DER SHA-256 指纹必须独立取得。脚本校验包、目录和输入，以普通服务 UID 初始化，再创建 systemd 配置。初始化失败保留受保护事务用于检查或重试，不覆盖已有数据库；安装成功保持停服，启用服务由用户执行。

管理员及会话存于 `state.sqlite3`，密码为 Argon2id 哈希。已有共享文件、标签库和操作状态保持独立业务含义。[运行合同](runtime-reference.md)、[运维](operations.md)。
