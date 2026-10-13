# xczs

## 项目简要介绍

轻量 Web 文件管理器，一个 Rust 进程提供管理页面和文件服务。

## 项目功能

- 文件浏览、搜索、上传续传、下载、移动、重命名和删除
- 文件标签管理、组合筛选和索引扫描
- 管理员认证、运行状态与结构化操作反馈

## 适用平台

Linux AMD64 GNU（`x86_64-unknown-linux-gnu`），内核需提供 `openat2`，并满足制品的 glibc 要求。公开访问应通过受控 HTTPS 入口，后端只能由可信入口访问；共享根由 xczs 独占写入。

## 如何部署

从[下载页](https://github.com/isarmg/xczs/releases)取得完整运行包，验证发布签名和摘要后解压。包内包含 `bin/xczs`、配置样例、systemd unit 和统一安装入口。

```sh
sha256sum --check SHA256SUMS
config="$(mktemp /tmp/xczs-config.XXXXXXXX)"
cp config/config.json.example "$config"
# 编辑 "$config"：明确共享根 serve_path，并核对 bind 和 data_dir。
systemd-ask-password 'Initial administrator password:' | sudo ./install.sh setup "$config" admin
sudo systemctl enable --now xczs.service
```

程序以普通 `isarmg-xczs` 账号运行。密码只从标准输入进入管理员状态，不写配置。安装器不递归修改共享文件的所有者。默认监听 `127.0.0.1:5000`，默认不信任转发头；有代理时明确配置其 IP/CIDR。

后续安装新运行包使用 `sudo ./install.sh upgrade`，保留配置、管理员和数据，校验成功后仅恢复原本活动的服务。

[详细文档](docs/README.md)
