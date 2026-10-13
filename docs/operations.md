# 运维

程序、配置、辅助工具和日志集中在固定 ROOT；共享文件和实例数据可外置，systemd 文件位于系统目录。普通服务账号 `isarmg-xczs` 只有必要可写目录权限，程序及签名资源由 root 管理。

```sh
sudo systemctl status xczs.service
sudo /opt/isarmg/xczs/maintenance.sh --root /opt/isarmg/xczs check
curl --fail http://127.0.0.1:5000/readyz
sudo /opt/isarmg/xczs/upgrade.sh --root /opt/isarmg/xczs --version VERSION --xssc /absolute/xssc
```

升级脚本验证固定产品、版本和签名后自动停服，XSSC 等待实际写入进程退出并取得维护锁，检查、备份、执行明确支持的数据步骤、替换程序、启动并验证实际版本和真实 readiness。只更新签名程序文件；不重新初始化、不改写现有配置、不丢弃新增数据。未知旧数据结构直接拒绝，不根据旧 JSON 或旧安装目录猜测迁移路径。

管理员改密可在管理后台完成。离线重置须停服，密码只经标准输入：

```sh
sudo systemctl stop xczs.service
systemd-ask-password 'New administrator password:' | sudo /opt/isarmg/xczs/maintenance.sh \
  --root /opt/isarmg/xczs admin-reset --username admin
sudo systemctl start xczs.service
```

重置保留管理员稳定 ID，修改会话版本并撤销旧会话。管理员凭据、会话和认证审计保存在 `state.sqlite3` 公共表；业务文件、`tags.db`、上传状态和操作记录保留。数据库使用 `DELETE` journal 与 `synchronous=EXTRA`，管理员连接使用相同模式，关闭时先排空连接池再排空状态线程。

日志位于 `ROOT/logs`，临时快照和运行文件在 `ROOT/run`。卸载由用户处理；不要通过删除管理员表、改数据库元数据或删除维护锁绕过故障。[故障排查](troubleshooting.md)。
