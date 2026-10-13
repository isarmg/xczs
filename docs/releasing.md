# 发布与签名包

当前正式发行资产为 `xczs-VERSION-linux-x86_64.tar.gz` 和 SHA256SUMS。包内 Ed25519 清单签名覆盖 server、xssi 安装维护脚本、libexec/setup、资源、状态合同和 systemd 模板。只有独立信任公钥及其 DER SHA-256 指纹能够授予安装和升级信任，checksum 仅验证传输完整性。

## 构建输入

使用固定 Rust、Node.js、Cargo/npm 锁图，以及 cargo-audit 0.22.2 和 cargo-cyclonedx 0.5.9；实际依赖 SBOM 进入签名 payload。config/xssi.lock 必须指向已审查的完整 xssi 提交。源码工作区必须干净，Cargo 版本对应的 annotated tag 必须精确指向 HEAD。打包器拒绝源码符号链接、submodule 和特殊文件，从目标 Git 对象生成私有快照，逐文件核对内容，再从快照构建内嵌 Web 的程序。构建脚本同时导出本次编译的身份、schema 和 Web 清单；server 不暴露资源或身份维护命令。

```sh
output="$(mktemp -d /tmp/xczs-release.XXXXXXXX)"
bash scripts/package-release.sh --output-dir "$output" --signing-key /absolute/independent-ed25519-private.pem
```

打包完成后再次核对原源码身份、tag 与干净状态，不覆盖已有资产。包内程序权限 0755、其他 payload 文件 0644、目录 0755，签名与哈希检查严格核对权限、大小、路径和额外条目。

## CI 与发行

主分支只读 CI 保留静态、Rust、真实运行、浏览器、覆盖率和部署布局检查。release-binary.yml 的只读 verify_build 从本次 tag 构建完整签名包，签名私钥来自 XSSI_PACKAGE_SIGNING_KEY secret，私钥只写入 0600 临时文件并在结束后删除。缺少签名输入直接失败，不产生未签名替代资产。

唯一 contents:write job 不 checkout、不编译、不执行候选程序，只下载本次 verify_build 的精确 artifact ID 并复核整组输入摘要后发布。不发布裸 ELF 安装资产，不覆写既有不同内容。

formal-release-e2e.yml 可手动执行临时密钥验收：独立源码 clone、完整签名包构建、外部测试公钥验签、逐文件权限/哈希核对、包校验及真实运行故障恢复测试。临时密钥不是生产信任根。正式发布前必须完成仓库质量检查，目标 systemd 的安装与升级验收由 xssi/XSSC 的工具测试负责。
