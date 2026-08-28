# Phase 3：Git 代码获取

LiteCI 当前已进入需求文档 Phase 3，提供受认证保护的项目同步接口。

## 已实现

- `GitService` 统一封装 Git 操作
- 所有 Git 进程调用经过 `CommandExecutor`
- 首次同步：`git clone --no-tags --single-branch`
- 已有工作区：`fetch --prune`、强制 checkout 与 reset
- 记录 repository、branch、commit SHA、commit message、author
- branch/ref 输入校验
- `POST /api/projects/{id}/sync`
- Sync API 要求有效 Bearer Session
- 工作区由服务端配置的 workspace root 管理，不接受请求直接指定文件系统路径

## 尚未实现

- Git Credential 表和凭证加密存储
- HTTPS Token 注入
- SSH Key 管理
- 实时 Git 日志持久化
- Pipeline Run / Stage Run
- 并发锁和取消任务管理
