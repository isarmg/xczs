# 安装与首次运行

完整安装步骤见[运行包安装手册](runtime-package.md)，二进制下载和源码构建见[项目 README](../README.md)。

统一使用普通 `isarmg-xczs` 服务账号：
- 程序 `/opt/isarmg/xczs/bin/xczs`
- 配置 `/etc/isarmg/xczs/config.json`，账号拥有的 `0600` 文件及 `0700` 父目录
- 状态 `/var/lib/isarmg/xczs`，权限 `0700`
- 运行目录 `/run/isarmg/xczs`，由 systemd 管理
- 共享根显式配置为绝对路径，例如 `/srv/xczs`；不要递归变更现有共享文件所有者

首次 `init --username admin` 从标准输入读取密码。已有实例只校验并运行，不重新初始化。升级保留已有配置。单监听 `bind` 同时包含 IP 和端口，生产默认 `127.0.0.1:5000`；HTTPS 由受控网关提供。

开发 HTTP 必须显式设置 `development: true` 且监听回环地址。不要用开发模式代替生产 HTTPS。
