# LiteCI

轻量级、单机优先的 Rust CI/CD 平台。当前实现是 `autoCI_需求与设计文档_v0.1.md` Phase 1 的第一个基础切片：基础 Web 服务、SQLite 迁移与最小 Project 数据模型。它不是完整的 Phase 1 验收版本。

尚未实现登录、Project CRUD、凭证、Runtime、Pipeline、Artifact、SSH/SFTP、部署、日志和历史；在认证与授权完成前不会开放管理和命令执行接口。

## 本地运行

```bash
cargo run
```

默认监听 `127.0.0.1:3000`，可通过 `AUTOCI_HOST`、`AUTOCI_PORT` 与 `AUTOCI_DATABASE_URL` 覆盖。

健康检查：

```bash
curl http://127.0.0.1:3000/health
```

## 验证

```bash
cargo check
cargo test
cargo build
```
