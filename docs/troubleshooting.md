# 故障排查

先记录程序版本、发生时间和操作 ID。下面的命令以默认安装路径为例，使用与服务相同的配置和用户。

## 启动或登录失败

```sh
sudo systemctl status xczs --no-pager --full
sudo journalctl -u xczs -n 100 --no-pager
sudo /opt/isarmg/xczs/maintenance.sh --root /opt/isarmg/xczs check
```

`check` 为离线诊断；若服务正在运行，先安排维护窗口正常停服。

| 症状 | 检查 | 正常结果与下一步 |
|---|---|---|
| 程序无法执行 | CPU、glibc/加载器、内核 `openat2` | 使用 Linux AMD64 GNU 匹配制品 |
| 配置校验失败 | 错误字段、文件权限、路径和 ACL | 按[权限要求](operations.md)修正后复查 |
| 状态未初始化 | 所选私有目录和实例来源 | 全新实例使用 xssi 安装脚本；已有当前实例使用原 ROOT 与状态目录 |
| `snapshot.busy` / `snapshot.source_changed` | 检查期间的正常写入 | 等待写入停止后重试只读校验；保留原状态 |
| 共享根或 Schema 不匹配 | 程序身份、根 dev/inode、数据库 | 使用同一当前实例的根和状态；停止写入调查其他情况 |
| 登录循环或来源错误 | HTTPS、规范 Host/Origin、Cookie、代理 | 浏览器通过同一 HTTPS 域名，后端只由网关访问 |
| 认证资源繁忙 | 登录正文读取与计算容量 | 稍后重试；核对服务全局容量和外部入口预算 |

## 上传、文件操作和标签

| 症状 | 检查 | 正常结果与下一步 |
|---|---|---|
| 磁盘空间不足或 507 | 共享根与状态盘的空间/inode，`min_free_space` | 留足预留空间后再提交 |
| 上传停滞或超时 | 网络、代理限制、idle/total timeout | 检查现有任务结果，再按界面继续或确认 |
| 操作结果 `unknown` | 操作 ID 对应任务与目标文件 | 先确认现有结果，再决定新操作；未知不代表未执行 |
| `move_state_conflict` 等 409 | 路径下活跃上传或回收任务 | 等待原任务完成，再以新操作身份提交 |
| 上传等待确认 | 原任务的暂存状态和目标版本 | 按界面选择发布或丢弃，保留相同任务上下文 |
| 标签缺失 | 扫描状态、文件是否被移动或替换 | 完整扫描后核对文件身份，必要时重新关联 |
| 发现 `.xczs-quarantine-*` | 隔离对象、日志、状态库 | 停服后调查；系统为保护身份不明的数据而保留对象 |

认证 API 客户端可用 `GET /__xczs__/api/jobs/<UUID>` 查询自己的 mutation job，读取 `job_id` 与 `running/succeeded/failed/unknown`。
详细状态和保留预算见[运行参考](runtime-reference.md)，完整请求流程见[工作流程](project-workflow.md)。
