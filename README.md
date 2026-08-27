# LiteCI

轻量级、单机优先的 Rust CI/CD 平台。当前实现正在推进 `LiteCI_需求与设计文档_v0.1.md` Phase 1：已具备基础 Web 服务、SQLite 迁移、存储初始化、首位管理员安全初始化和登录会话。它不是完整的 Phase 1 验收版本。

尚未实现 Project CRUD、鉴权中间件、CSRF、凭证、Runtime、Pipeline、Artifact、SSH/SFTP、部署、日志和历史；在鉴权中间件完成前不会开放管理和命令执行接口。

## 本地运行

```bash
cargo run
```

默认监听 `127.0.0.1:3000`，可通过 `LITECI_HOST`、`LITECI_PORT` 与 `LITECI_DATABASE_URL` 覆盖。

从 autoCI 原路径升级时，如果当前目录存在 `autoci.db` 且尚无 `liteci.db`，LiteCI 会继续使用原数据库，避免更名导致数据不可见。确认升级正常后可停机将文件及对应的 `-wal`、`-shm` 文件统一改名，或显式设置 `LITECI_DATABASE_URL`。

首次管理员初始化接口仅接受来自本机回环地址的请求；请在 LiteCI 所在主机上完成首次初始化。

健康检查：

```bash
curl http://127.0.0.1:3000/health
```

## 验证

```bash
cargo fmt --all -- --check
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build
```
