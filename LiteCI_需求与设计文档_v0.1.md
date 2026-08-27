# auto CI — 需求与设计文档

> 文档版本：v0.1  
> 项目定位：轻量级、自建、面向中小团队的 CI/CD 与部署平台  
> 技术方向：Rust + Web + SQLite + Git + SSH/SFTP + Docker  
> 核心原则：**简单、轻量、可维护、逐步演进，不追求重写 GitLab**

---

# 1. 项目概述

## 1.1 项目背景

现有 GitLab 功能完整，但对于个人/小团队自建环境而言，资源占用和系统复杂度较高。

Forgejo 适合作为轻量 Git 平台，但本项目实际核心需求并不是完整 Git 平台，而是：

```text
Git 仓库
   ↓
检测触发条件
   ↓
获取代码
   ↓
准备开发环境
   ↓
Install
   ↓
Build
   ↓
Test（可选）
   ↓
Docker Build（可选）
   ↓
生成产物
   ↓
部署目标环境
   ↓
备份
   ↓
重启
   ↓
健康检查
   ↓
完成
```

因此，本项目不以“Rust 重写 GitLab”为目标，而是构建一个：

> **轻量级 Web CI/CD + 构建 + 部署平台。**

代码仓库仍然使用 Gitee、GitHub、GitLab、Forgejo 或其他标准 Git 服务，本系统不负责第一阶段的 Git 仓库托管。

---

# 2. 产品定位

## 2.1 核心定位

LiteCI 是一个：

- 单机优先
- Web 管理
- Rust 开发
- 低资源占用
- 支持多项目
- 支持多环境
- 支持自动触发
- 支持可选 Pipeline Stage
- 支持远程服务器部署
- 支持构建产物管理
- 支持版本发布与回滚

的轻量级 DevOps 平台。

## 2.2 非目标

第一阶段明确不做：

- Git 仓库托管
- Git 服务协议实现
- Issue
- Pull Request / Merge Request
- Wiki
- Docker Registry
- Kubernetes
- LDAP / SAML / 企业 SSO
- 多租户复杂权限
- 分布式调度
- 高可用集群
- 微服务
- 自研 Git 对象数据库
- 自研 Git HTTP/SSH Server

系统通过调用服务器已有的 `git`、`ssh`、`sftp`、`docker` 等工具完成相关工作。

---

# 3. 核心设计原则

## 3.1 简单优先

能通过操作系统原生命令完成的事情，不重复造轮子。

例如：

```text
Git       → git
SSH       → ssh
SFTP      → sftp / 系统 SSH 能力
Docker    → docker
定时任务  → Tokio scheduler / 系统服务
```

## 3.2 单体优先

第一阶段使用一个 Rust Web 服务：

```text
jzx-devops
```

不要拆微服务。

## 3.3 配置驱动

用户通过 Web 页面配置项目、Runtime、Pipeline、Trigger、Environment。

不要要求用户必须修改 Shell 文件。

## 3.4 脚本可扩展

虽然平台提供标准化 Stage，但任何 Stage 都允许配置自定义命令或脚本。

## 3.5 构建与部署分离

Build 产生 Artifact。

Deploy 使用指定 Artifact。

不要为了重新部署而重新 Build。

## 3.6 环境隔离

DEV、TEST、UAT、PROD 分开配置。

环境变量、服务器、目录、部署方式等均独立。

## 3.7 安全优先

所有远程服务器原则上使用专用 `deploy` 用户，不使用 root。

Pipeline 执行命令属于高权限操作，后续必须支持 Runner / Sandbox 隔离。

---

# 4. 用户角色

第一阶段只实现简单角色模型：

## 4.1 管理员

可以：

- 管理用户
- 管理项目
- 管理 Runtime
- 管理服务器
- 管理凭证
- 管理环境
- 管理 Pipeline
- 查看所有执行记录

## 4.2 普通用户

可以：

- 查看授权项目
- 手动执行 Pipeline
- 查看日志
- 查看构建产物
- 执行授权环境部署

第一阶段可以暂时只有管理员角色，但数据库设计应预留角色字段。

---

# 5. 系统总体架构

```text
                    ┌──────────────────────┐
                    │      Gitee           │
                    │ GitHub / GitLab ...  │
                    └──────────┬───────────┘
                               │
                         Git / Polling
                         Webhook（后续）
                               │
                               ▼
┌──────────────────────────────────────────────────┐
│                  LiteCI                      │
│                                                  │
│  ┌───────────┐   ┌───────────┐                  │
│  │ Project   │   │ Trigger   │                  │
│  └───────────┘   └─────┬─────┘                  │
│                        │                         │
│  ┌───────────┐   ┌─────▼─────┐                  │
│  │ Runtime   │   │ Pipeline  │                  │
│  └───────────┘   └─────┬─────┘                  │
│                        │                         │
│                 ┌──────▼──────┐                 │
│                 │ Job Executor│                 │
│                 └──────┬──────┘                 │
│                        │                         │
│          ┌─────────────┼─────────────┐           │
│          ▼             ▼             ▼           │
│        Git           Shell         Docker        │
│                                                  │
│  ┌──────────────┐       ┌──────────────────┐    │
│  │ Artifact     │       │ Deployment Engine │    │
│  └──────────────┘       └─────────┬────────┘    │
└────────────────────────────────────┼─────────────┘
                                     │
                              SSH / SFTP
                                     │
              ┌──────────────────────┼────────────────────┐
              ▼                      ▼                    ▼
             DEV                    TEST                 UAT
                                                            │
                                                            ▼
                                                           PROD
```

---

# 6. 推荐技术栈

## 6.1 Backend

Rust。

推荐：

- Tokio：异步运行时
- Axum：Web/API
- SQLx：数据库访问
- SQLite：第一阶段数据库
- Serde：JSON/YAML/配置解析
- tracing：日志
- uuid：实体 ID
- chrono / time：时间
- anyhow / thiserror：错误处理

## 6.2 Frontend

第一阶段优先选择简单方案：

- 服务端渲染优先
- 或轻量前端框架

不要求 React/Vue。

如果需要交互增强，可以使用少量 JavaScript / HTMX 类方案。

目标：

> 不为了一个管理后台引入大型前端工程。

## 6.3 数据库

第一阶段：

```text
SQLite
```

要求：

- WAL 模式
- migration
- 外键约束
- 事务
- 数据库备份

未来如果需要多实例/高并发，可以考虑 PostgreSQL。

---

# 7. 项目模型

一个 Project 是 CI/CD 的核心配置单位。

示例：

```text
项目名称：jzx-website
Git URL：
https://gitee.com/chenbin0117/jzx-wesite.git

默认分支：
main

语言：
Node.js

框架：
Vue

状态：
启用
```

## 7.1 Project 基本字段

建议：

```text
id
name
description
git_url
git_auth_id
default_branch
runtime_id
status
workspace_path
created_at
updated_at
```

---

# 8. Git 代码源

## 8.1 支持

第一阶段：

- HTTPS
- SSH

例如：

```text
https://gitee.com/xxx/project.git
```

或：

```text
git@gitee.com:xxx/project.git
```

## 8.2 Git 认证

支持：

### 无认证

适用于公开仓库。

### HTTPS Token

```text
username
token
```

### SSH Key

```text
private_key
public_key
```

凭证不能直接显示。

---

# 9. Git 获取代码策略

系统不负责托管 Git 仓库。

Pipeline 执行时：

```text
创建 Workspace
      ↓
git clone / git fetch
      ↓
checkout 指定 branch/tag/commit
      ↓
记录 Commit SHA
```

推荐优先使用：

```text
git fetch
git checkout
```

而不是每次完整 clone，以减少下载量。

Pipeline 必须记录：

```text
repository
branch
tag
commit_sha
commit_message
author
```

---

# 10. Workspace

每次 Pipeline Run 使用独立工作目录。

例如：

```text
/data/jzx-devops/workspaces/
    jzx-website/
        run-1001/
        run-1002/
```

执行完成后根据配置：

```text
保留
或
自动清理
```

建议默认自动清理旧 Workspace。

---

# 11. Runtime / 开发环境管理

## 11.1 第一阶段支持

- Node.js
- Python
- Rust

Vue 不作为 Runtime。

Vue 属于项目框架，主要依赖 Node.js。

例如：

```text
Runtime：
Node.js 22

Framework：
Vue 3
```

## 11.2 多版本

支持同一 Runtime 多版本。

例如：

```text
Node.js:
18
20
22
24

Python:
3.10
3.11
3.12
3.13

Rust:
stable
1.86
1.87
```

## 11.3 Runtime Manager

不要把 Runtime 版本管理逻辑写死在 Pipeline Engine 中。

抽象：

```text
Runtime
 ├── name
 ├── version
 ├── manager
 └── activation_command
```

例如：

```text
Node.js → fnm
Python  → uv / venv
Rust    → rustup
```

第一阶段允许系统管理员配置 Runtime 初始化脚本。

例如：

```bash
source ~/.bashrc
fnm use 22
```

或者：

```bash
source ~/.cargo/env
rustup default 1.87
```

---

# 12. Pipeline

Pipeline 是项目执行流程。

Pipeline 不应固定为：

```text
Clone → Install → Build → Test → Docker → Deploy
```

而应该是可选 Stage。

示例：

```text
✓ Clone
✓ Runtime
✓ Install
✓ Build
□ Test
□ Docker Build
✓ Package
✓ Deploy
✓ Verify
```

## 12.1 Stage 顺序

第一阶段采用串行执行。

例如：

```text
Clone
 ↓
Runtime
 ↓
Install
 ↓
Build
 ↓
Test
 ↓
Docker Build
 ↓
Package
 ↓
Deploy
 ↓
Verify
```

未来再考虑并行/DAG。

---

# 13. Stage 类型

第一阶段建议支持：

```text
CLONE
RUNTIME
INSTALL
BUILD
TEST
PACKAGE
DOCKER_BUILD
DEPLOY
VERIFY
CUSTOM
```

---

# 14. Install Stage

用户配置：

```text
启用：是

命令：
npm ci
```

其他项目可以：

```bash
pnpm install --frozen-lockfile
```

Python：

```bash
pip install -r requirements.txt
```

Rust：

```bash
cargo fetch
```

命令不写死。

---

# 15. Build Stage

工作目录默认：

```text
项目根目录
```

用户配置：

```text
Build Command：
npm run build
```

系统执行：

```bash
cd /workspace
npm run build
```

## 15.1 产物配置

支持：

```text
产物名称：
dist

路径：
dist/
```

也支持：

```text
target/release/app
```

或：

```text
dist/**
```

---

# 16. Artifact

Build 完成后生成 Artifact。

目录建议：

```text
/data/jzx-devops/artifacts/
    jzx-website/
        1001/
            metadata.json
            dist.tar.gz
```

metadata 至少记录：

```text
project
run_id
commit_sha
branch
tag
version
created_at
file
size
checksum
```

Artifact 应具备：

- 下载
- 删除
- 查看
- 部署
- 回滚

能力。

---

# 17. Test Stage

完全可选。

例如：

```bash
npm test
```

或者：

```bash
cargo test
```

或者：

```bash
pytest
```

没有测试代码的项目：

```text
Test = Disabled
```

Pipeline 自动跳过。

---

# 18. Docker Build Stage

完全可选。

配置：

```text
启用：是

Dockerfile：
./Dockerfile

Image Name：
jzx-website

Tag：
自动生成
```

Tag 推荐：

```text
v1.0.0
```

或者：

```text
jzx-website:run-1001
jzx-website:a81d32f
```

第一阶段不强制 Docker Registry。

可以：

```text
docker build
      ↓
docker save
      ↓
image.tar
      ↓
SFTP
      ↓
docker load
```

---

# 19. Trigger 触发系统

Trigger 是本项目核心功能之一。

支持：

1. 手动触发
2. Branch Push
3. Branch Created
4. Tag Created / Tag 更新
5. 定时触发
6. 后续 Webhook

---

# 20. Branch Trigger

支持通配符。

例如：

```text
main
master
dev-*
release/*
feature/*
*
```

配置：

```text
Trigger Type:
Branch

Pattern:
release/*
```

匹配：

```text
release/v1.0
release/v2.0
```

---

# 21. Tag Trigger

这是重要功能。

例如：

```text
Pattern:
v*
```

当发现：

```text
v1.0.0
```

执行 Pipeline。

下一次：

```text
v1.0.1
```

再次执行。

推荐支持 glob：

```text
v*
v1.*
v1.0.*
release-*
```

---

# 22. Branch Created Trigger

支持检测新建分支。

例如：

```text
Pattern:
release/*
```

创建：

```text
release/v1.0
```

触发一次。

系统需要保存已见过的 ref 状态，避免重复触发。

---

# 23. Tag Created Trigger

例如：

```text
Pattern:
v*
```

检测到新 Tag：

```text
v1.0.0
```

触发。

必须避免同一个 Tag 在每次 polling 时重复执行。

---

# 24. Push Trigger

支持：

```text
Push
Branch Pattern:
main
```

例如：

```text
main
```

每次发现 commit SHA 发生变化，执行一次。

---

# 25. 定时 Trigger

支持：

```text
每 5 分钟
每 30 分钟
每天 02:00
每周一 03:00
Cron
```

推荐底层统一使用 Cron 表达式。

例如：

```text
0 2 * * *
```

---

# 26. Trigger Polling

第一阶段不强制依赖 Webhook。

系统 Scheduler 定期检查远程 Git：

```text
Scheduler
    ↓
git ls-remote
    ↓
读取 branches/tags
    ↓
与历史状态比较
    ↓
发现变化
    ↓
创建 Pipeline Run
```

这样即使 Gitee 无法访问内网服务器，也可以工作。

Polling 周期可配置：

```text
1 min
5 min
10 min
30 min
```

默认：

```text
5 min
```

后续增加 Webhook 后：

```text
Webhook = 实时触发
Polling = 兜底
```

---

# 27. Trigger 与 Pipeline 解耦

一个项目可以拥有多个 Trigger。

例如：

```text
Trigger #1
Branch: main
→ TEST

Trigger #2
Branch: release/*
→ UAT

Trigger #3
Tag: v*
→ PROD

Trigger #4
Schedule: 0 2 * * *
→ TEST
```

Trigger 不直接定义所有 Pipeline 内容。

建议关系：

```text
Trigger
   ↓
Pipeline
   ↓
Environment
```

---

# 28. 手动执行

项目页面：

```text
[立即执行]
```

选择：

```text
Branch / Tag / Commit
Environment
```

例如：

```text
Branch:
main

Environment:
TEST
```

点击：

```text
[开始执行]
```

---

# 29. Environment

第一阶段默认：

```text
DEV
TEST
UAT
PROD
```

用户可以增加：

```text
DEMO
STAGING
PRE
```

---

# 30. Environment 配置

每个环境独立：

```text
名称
服务器
SSH 用户
部署目录
.env 路径
服务名称
部署脚本
验证脚本
环境变量
```

例如 TEST：

```text
Server:
192.168.1.20

User:
deploy

Deploy Path:
/data/jzx-website

Env File:
/data/jzx-website/.env.test

Service:
jzx-website
```

---

# 31. 环境变量

分为：

### 项目变量

所有环境可用。

### 环境变量

仅当前环境。

### Secret

敏感信息。

例如：

```text
NODE_ENV
API_BASE_URL
DATABASE_URL
JWT_SECRET
```

Secret 不允许明文显示。

---

# 32. Secret 管理

第一阶段：

- 数据库存储加密后的 Secret
- 主密钥从环境变量读取
- 数据库不直接保存明文
- Web 页面只显示 `******`
- Pipeline 日志自动脱敏

例如：

```text
TOKEN=abcdef
```

日志中：

```text
TOKEN=******
```

---

# 33. Server 管理

服务器页面：

```text
服务器
├── TEST-01
├── UAT-01
└── PROD-01
```

服务器配置：

```text
名称
IP / Host
SSH Port
User
SSH Credential
部署目录
备注
```

---

# 34. 一键建立 SSH 信任

用户点击：

```text
[建立信任]
```

系统：

```text
生成/选择 SSH Key
       ↓
连接服务器
       ↓
写入 authorized_keys
       ↓
测试 SSH
       ↓
测试 SFTP
```

完成后显示：

```text
✓ SSH
✓ SFTP
```

---

# 35. SSH 安全要求

不建议 root。

默认：

```text
deploy
```

服务器侧应仅授予必要权限。

例如：

```text
/data/jzx-website
docker
必要的 systemctl
```

后续可以支持 sudo 白名单。

---

# 36. Deployment

标准部署流程：

```text
Backup
 ↓
Pre Deploy
 ↓
Upload Artifact
 ↓
Deploy
 ↓
Restart
 ↓
Verify
 ↓
Post Deploy
```

每一步均可启用/禁用。

---

# 37. Backup

例如：

```bash
tar czf \
/backup/jzx-website-20260827-110000.tar.gz \
/data/jzx-website
```

备份文件应记录：

```text
environment
project
run_id
timestamp
```

支持保留策略：

```text
保留最近 5 个
保留最近 10 个
保留 30 天
```

第一阶段至少实现：

```text
保留最近 N 个
```

---

# 38. Upload

支持：

```text
SFTP
```

上传：

```text
Artifact
```

目标：

```text
服务器:/tmp/jzx-devops/run-1001/
```

不要直接覆盖正式目录。

---

# 39. Deploy

可以使用平台内置部署方式：

```text
解压
复制
docker load
docker compose
```

也允许自定义：

```bash
./deploy.sh
```

---

# 40. Restart

支持：

```text
systemctl restart xxx
```

或者：

```text
docker compose up -d
```

或者：

```text
docker restart xxx
```

也支持自定义脚本。

---

# 41. Verify

支持：

## HTTP

```text
GET http://127.0.0.1:8080/health
```

要求：

```text
HTTP 200
```

## TCP

检查：

```text
host:port
```

## Shell

执行：

```bash
curl http://localhost/health
```

## Custom Script

```bash
./verify.sh
```

验证失败：

```text
Pipeline = FAILED
```

---

# 42. Pipeline Run

每一次执行都是一个 Run。

例如：

```text
Run #1001

Project:
jzx-website

Branch:
main

Commit:
a81d32f

Trigger:
Push

Environment:
TEST

Status:
SUCCESS
```

---

# 43. Run 状态

定义：

```text
PENDING
RUNNING
SUCCESS
FAILED
CANCELLED
SKIPPED
```

---

# 44. Stage 状态

同样：

```text
PENDING
RUNNING
SUCCESS
FAILED
CANCELLED
SKIPPED
```

Pipeline 总状态由 Stage 汇总。

---

# 45. 日志系统

必须实时查看。

页面：

```text
Pipeline #1001

✓ Clone        3s
✓ Runtime      1s
✓ Install     30s
✓ Build       20s
✓ Test        10s
✓ Package      3s
✓ Deploy      15s
✓ Verify       2s
```

点击 Stage：

```text
> npm ci

added 421 packages

> npm run build

vite building...
...
```

日志支持：

- 实时追加
- 自动滚动
- 搜索
- 下载
- 按 Stage 查看

---

# 46. 日志存储

第一阶段不需要 Elasticsearch。

直接：

```text
/data/jzx-devops/logs/
```

例如：

```text
logs/
  project-1/
    run-1001/
      pipeline.log
      clone.log
      build.log
      deploy.log
```

数据库保存日志路径与摘要。

---

# 47. Pipeline 历史

项目页面显示：

```text
#1001  main      a81d32f   TEST   SUCCESS
#1000  main      b8212ac   TEST   SUCCESS
#999   v1.0.0    c8129aa   PROD   SUCCESS
#998   release/1 UAT        FAILED
```

支持：

- 查看
- 重试
- 取消
- 下载日志
- 查看 Artifact
- 再次部署

---

# 48. Tag / Release / Version

Tag 是版本的重要来源。

例如：

```text
v1.0.0
```

自动识别：

```text
version = 1.0.0
```

Artifact：

```text
jzx-website-1.0.0.tar.gz
```

部署记录：

```text
PROD
Version: 1.0.0
Commit: a81d32f
Run: #999
```

---

# 49. 回滚

由于 Artifact 持久化，因此：

```text
PROD
当前版本：v1.0.1

历史：

v1.0.1
v1.0.0
v0.9.8
```

点击：

```text
[回滚到 v1.0.0]
```

直接使用历史 Artifact。

不重新 Build。

---

# 50. 生产环境审批

生产部署建议支持：

```text
Build
 ↓
Test
 ↓
[批准生产]
 ↓
Deploy PROD
```

第一阶段可以简单实现：

```text
Deploy PROD
```

点击时二次确认：

```text
确认部署：
jzx-website
Version: v1.0.1
Environment: PROD

[取消] [确认部署]
```

后续再实现完整 Approval。

---

# 51. Pipeline 配置示例

一个 Vue 项目：

```text
Project:
jzx-website

Runtime:
Node.js 22

Install:
pnpm install --frozen-lockfile

Build:
pnpm build

Artifact:
dist/

Test:
Disabled

Docker:
Disabled

Deploy:
Enabled
```

流程：

```text
Clone
 ↓
Runtime
 ↓
Install
 ↓
Build
 ↓
Package
 ↓
Deploy
 ↓
Verify
```

---

# 52. Tag 发布示例

Trigger：

```text
Type:
Tag

Pattern:
v*
```

用户：

```bash
git tag v1.0.0
git push origin v1.0.0
```

系统：

```text
检测 Tag
 ↓
Pipeline #1002
 ↓
Checkout v1.0.0
 ↓
Install
 ↓
Build
 ↓
Package
 ↓
Deploy PROD
 ↓
Verify
```

---

# 53. 数据模型建议

第一阶段主要表：

```text
users
projects
git_credentials
runtimes
runtime_versions
pipeline_configs
pipeline_stages
triggers
environments
servers
server_credentials
environment_variables
secrets
pipeline_runs
stage_runs
artifacts
deployment_records
scheduler_jobs
audit_logs
```

---

# 54. 关键关系

```text
Project
 ├── GitCredential
 ├── Runtime
 ├── Pipeline
 │    └── Stages
 ├── Triggers
 └── Environments
       └── Server

Pipeline
 └── PipelineRun
       ├── StageRuns
       ├── Artifacts
       └── DeploymentRecord
```

---

# 55. API 设计

第一阶段 Web API 可以采用 REST。

## Project

```http
GET    /api/projects
POST   /api/projects
GET    /api/projects/:id
PUT    /api/projects/:id
DELETE /api/projects/:id
```

## Pipeline

```http
GET  /api/projects/:id/pipeline
PUT  /api/projects/:id/pipeline
POST /api/projects/:id/runs
GET  /api/projects/:id/runs
GET  /api/runs/:id
POST /api/runs/:id/cancel
POST /api/runs/:id/retry
```

## Trigger

```http
GET    /api/projects/:id/triggers
POST   /api/projects/:id/triggers
PUT    /api/triggers/:id
DELETE /api/triggers/:id
```

## Environment

```http
GET    /api/environments
POST   /api/environments
PUT    /api/environments/:id
DELETE /api/environments/:id
```

## Server

```http
GET  /api/servers
POST /api/servers
POST /api/servers/:id/test
POST /api/servers/:id/bootstrap
```

---

# 56. Web 页面

第一阶段页面：

```text
登录
首页 Dashboard

项目
 ├── 项目列表
 ├── 项目详情
 ├── Pipeline
 ├── Trigger
 ├── Artifact
 └── 执行历史

Runtime
 ├── Runtime 列表
 └── Runtime 版本

服务器
 ├── 服务器列表
 └── SSH 配置

环境
 ├── DEV
 ├── TEST
 ├── UAT
 └── PROD

系统设置
 ├── 用户
 ├── 凭证
 └── 系统配置
```

---

# 57. Dashboard

首页展示：

```text
项目数量
运行中的 Pipeline
今日成功
今日失败
最近执行
最近部署
```

例如：

```text
项目：8
运行中：1
今日成功：12
今日失败：1
```

---

# 58. 项目详情页

推荐布局：

```text
jzx-website

[概览] [Pipeline] [Triggers] [Artifacts] [部署] [设置]

最新状态：
✓ SUCCESS

Branch：
main

Commit：
a81d32f

最近版本：
v1.0.1

[立即执行]
```

---

# 59. Pipeline 配置 UI

建议采用可视化 Stage 列表：

```text
┌─────────────────────────────┐
│ Pipeline                    │
├─────────────────────────────┤
│ ☑ Clone                     │
│ ☑ Runtime                   │
│ ☑ Install                   │
│ ☑ Build                     │
│ ☐ Test                      │
│ ☐ Docker Build              │
│ ☑ Package                   │
│ ☑ Deploy                    │
│ ☑ Verify                    │
└─────────────────────────────┘
```

每个 Stage 点击后配置：

```text
名称
启用
命令
工作目录
环境变量
失败处理
```

---

# 60. 自定义 Stage

必须支持：

```text
CUSTOM
```

例如：

```bash
./scripts/generate-version.sh
```

或者：

```bash
python scripts/check.py
```

这样可以避免平台功能不够时必须修改平台代码。

---

# 61. 命令执行器设计

Rust 内部统一封装：

```text
CommandExecutor
```

负责：

- 启动进程
- 设置环境变量
- 设置工作目录
- 捕获 stdout
- 捕获 stderr
- 实时写日志
- 返回 exit code
- timeout
- cancellation

不要让各模块直接散落使用 `Command`。

---

# 62. Pipeline Engine

核心接口概念：

```text
PipelineEngine
    ↓
PipelineRun
    ↓
StageExecutor
```

伪代码：

```text
load pipeline
create run
for stage in stages:
    if disabled:
        skip
    else:
        execute stage
        if failed:
            stop
create final status
```

---

# 63. Cancellation

用户可以：

```text
[取消运行]
```

系统应：

```text
发送终止信号
 ↓
停止当前进程
 ↓
标记 Run = CANCELLED
```

必须处理子进程，不能只杀 Rust 父进程。

---

# 64. Timeout

每个 Stage 支持：

```text
Timeout
```

例如：

```text
Install:
10 min

Build:
10 min

Test:
20 min

Deploy:
10 min
```

超时：

```text
FAILED
```

并终止对应进程。

---

# 65. 并发控制

第一阶段：

```text
单服务器全局最多 N 个 Pipeline
```

默认：

```text
1
```

项目可以配置：

```text
允许并发：
否
```

这样同一个项目不会出现：

```text
Run #100
Run #101
```

同时部署生产。

---

# 66. Deployment Lock

对于 PROD：

```text
同一个项目同一时间只能有一个生产部署。
```

避免：

```text
v1.0.0 正在部署
v1.0.1 又开始部署
```

造成覆盖。

---

# 67. Audit Log

记录关键操作：

```text
登录
创建项目
修改 Pipeline
修改 Trigger
添加服务器
建立 SSH 信任
执行 Pipeline
部署 TEST
部署 PROD
回滚
删除 Artifact
```

至少记录：

```text
user
action
target
timestamp
ip
result
```

---

# 68. 配置文件

程序本身提供：

```text
config.toml
```

例如：

```toml
[server]
host = "0.0.0.0"
port = 3000

[database]
url = "sqlite:///data/jzx-devops.db"

[storage]
workspace = "/data/jzx-devops/workspaces"
artifacts = "/data/jzx-devops/artifacts"
logs = "/data/jzx-devops/logs"

[scheduler]
enabled = true
```

Secret 不写入 Git。

---

# 69. 部署方式

第一阶段支持：

## 直接运行

```bash
./jzx-devops
```

## systemd

```text
jzx-devops.service
```

后续可以提供 Docker 镜像。

---

# 70. 推荐目录结构

服务器：

```text
/data/jzx-devops/
├── jzx-devops.db
├── config/
├── workspaces/
├── artifacts/
├── logs/
├── backups/
└── cache/
```

程序：

```text
/opt/jzx-devops/
└── jzx-devops
```

---

# 71. Backup

数据库必须可以备份。

至少支持：

```text
SQLite 数据库备份
Artifact 备份
配置备份
```

建议提供：

```text
[立即备份]
```

---

# 72. 安全边界

必须重点考虑：

> Pipeline 本质上可以执行 Shell。

因此第一阶段如果只用于可信内部项目，可以允许本机执行。

但代码架构必须为未来 Runner 留出边界：

```text
LiteCI Server
       ↓
Job
       ↓
Runner
       ↓
Docker Sandbox
```

未来 Runner 执行不可信代码。

---

# 73. 第一阶段安全策略

由于目标是局域网自用：

- Web 默认监听局域网
- 登录认证
- CSRF 防护
- 密码使用 Argon2 等安全哈希
- Secret 加密
- 日志脱敏
- SSH Key 权限控制
- 禁止 root 作为默认部署用户
- Pipeline 命令执行记录
- 所有生产操作写 Audit Log

---

# 74. 第一阶段开发范围 V0.1

必须完成：

```text
✓ Rust Web Server
✓ SQLite
✓ 用户登录
✓ Project
✓ Git URL
✓ Git Credential
✓ 手动获取代码
✓ Node.js Runtime
✓ Python Runtime
✓ Rust Runtime
✓ Install
✓ Build
✓ Artifact
✓ DEV/TEST/UAT/PROD
✓ Server
✓ SSH
✓ SFTP
✓ Deploy
✓ Verify
✓ Pipeline Log
✓ Pipeline History
```

完成标准：

> 能够通过 Web 页面创建一个 Node/Vue 项目，并完成：

```text
Gitee
 ↓
Clone
 ↓
Node.js 22
 ↓
pnpm install
 ↓
pnpm build
 ↓
dist/
 ↓
SFTP
 ↓
TEST
 ↓
Restart
 ↓
HTTP Health Check
```

---

# 75. V0.2

加入：

```text
✓ Branch Trigger
✓ Tag Trigger
✓ Branch Created
✓ Tag Created
✓ Polling
✓ Schedule
✓ 手动指定 Branch/Tag
```

完成后实现：

```text
git push main
 ↓
自动 Build
```

以及：

```text
git tag v1.0.0
 ↓
自动 Build
 ↓
自动 Deploy
```

---

# 76. V0.3

加入：

```text
✓ Test Stage
✓ Docker Build
✓ Docker Deploy
✓ Backup
✓ Pre Deploy
✓ Post Deploy
✓ 健康检查增强
✓ Artifact 版本管理
```

---

# 77. V0.4

加入：

```text
✓ Production Approval
✓ 一键回滚
✓ Environment Variables
✓ Secret
✓ Audit Log
✓ Deployment History
```

---

# 78. V0.5

根据实际使用情况决定是否开发：

```text
○ Webhook
○ YAML Pipeline
○ Runner
○ Docker Sandbox
○ 多构建节点
○ 并行 Pipeline
```

这些不是第一阶段必须功能。

---

# 79. YAML Pipeline

不要第一版实现。

只有当 Web UI Pipeline 配置无法满足实际需求时，再增加：

```text
.jzx-devops.yml
```

例如：

```yaml
runtime:
  node: "22"

stages:
  - install
  - build
  - test
  - package
  - deploy

install:
  command: pnpm install --frozen-lockfile

build:
  command: pnpm build

test:
  command: pnpm test

package:
  artifacts:
    - dist/

deploy:
  environment: test
```

原则：

> Web UI 是默认配置方式，YAML 是高级配置方式。

---

# 80. Webhook

后续支持：

```text
POST /api/webhooks/:project_id
```

Gitee/GitHub/GitLab 推送：

```text
Push
Tag
Branch
```

系统收到事件后直接创建 Run。

Polling 仍然保留作为兜底。

---

# 81. Runner

当出现以下需求时再开发：

- 构建机与 LiteCI 不在同一台服务器
- 多台构建服务器
- 构建任务互相影响
- 需要 Docker 隔离
- 需要高并发

架构：

```text
LiteCI
     ↓
Job Queue
     ↓
Runner
     ↓
Workspace
     ↓
Command
```

---

# 82. Runner 协议预留

第一版 Pipeline Engine 不要把执行逻辑写死在 Web Handler。

抽象：

```text
Executor
├── LocalExecutor
└── RemoteExecutor（未来）
```

这样未来增加 Runner 不需要重写 Pipeline。

---

# 83. 完整业务流程

## 自动部署

```text
Gitee
  ↓
Polling
  ↓
发现 main 新 Commit
  ↓
创建 Pipeline Run
  ↓
Clone
  ↓
Runtime
  ↓
Install
  ↓
Build
  ↓
Package
  ↓
Deploy TEST
  ↓
Restart
  ↓
Verify
  ↓
SUCCESS
```

---

# 84. Tag 发布流程

```text
开发完成
  ↓
git tag v1.0.0
  ↓
push tag
  ↓
Polling 检测
  ↓
匹配 v*
  ↓
Pipeline
  ↓
Build
  ↓
Artifact v1.0.0
  ↓
Deploy PROD
  ↓
Verify
  ↓
SUCCESS
```

---

# 85. 失败流程

例如 Build 失败：

```text
Build
 ↓
exit code != 0
 ↓
Stage = FAILED
 ↓
Pipeline = FAILED
 ↓
停止后续 Stage
 ↓
保存日志
 ↓
通知/页面显示
```

默认：

> 任一 Stage 失败，则停止后续 Stage。

未来支持：

```text
continue_on_failure
```

---

# 86. 重试

Pipeline 支持：

```text
[重新执行]
```

默认使用：

```text
相同 Commit
相同 Pipeline 配置
```

也支持：

```text
重新执行失败 Stage
```

后者可以后续实现。

---

# 87. 重新部署 Artifact

非常重要：

```text
Artifact #1001
```

可以直接：

```text
[部署 TEST]
[部署 UAT]
[部署 PROD]
```

不需要重新 Build。

这也是 Build 与 Deploy 分离的核心价值。

---

# 88. 项目配置继承

未来可以支持：

```text
全局 Runtime
    ↓
项目 Runtime
    ↓
环境 Runtime
```

第一阶段不要复杂化。

只需要：

```text
Project → Runtime
Project → Environment
```

---

# 89. 非功能需求

## 性能

目标不是高并发。

单机支持：

```text
10～50 个项目
```

并能稳定运行。

## 内存

目标：

> 正常空闲状态尽量控制在几十 MB 级别。

具体数值以实际实现为准，不以绝对数字作为硬性验收条件。

## 启动速度

目标：

```text
秒级启动
```

## 部署

支持：

```text
Linux x86_64
```

后续：

```text
ARM64
```

---

# 90. 可观测性

程序提供：

```text
INFO
WARN
ERROR
DEBUG
```

使用：

```text
tracing
```

日志至少包括：

```text
timestamp
level
module
project_id
run_id
message
```

---

# 91. 错误处理

不要：

```rust
unwrap()
```

作为业务错误处理方式。

统一：

```text
Result<T, E>
```

用户可见错误：

```text
Git Clone 失败
SSH 连接失败
Build 失败
Artifact 不存在
服务器不可达
Health Check 失败
```

同时记录详细内部日志。

---

# 92. 数据库迁移

使用 migration。

禁止开发过程中直接依赖：

```text
手工修改数据库
```

每次数据库结构变化：

```text
migration
```

保证新旧版本可迁移。

---

# 93. 测试要求

第一阶段至少：

## Unit Test

测试：

```text
Trigger Pattern
Cron
Pipeline 状态
Artifact
配置解析
```

## Integration Test

测试：

```text
Git
Command Executor
Pipeline
```

## E2E

至少实现一个完整项目：

```text
Git
 ↓
Build
 ↓
Artifact
 ↓
Deploy
 ↓
Verify
```

---

# 94. Trigger 匹配规则测试

必须覆盖：

```text
Pattern: v*
v1.0.0      → MATCH
v2.0.0      → MATCH
release-1   → NO MATCH

Pattern: release/*
release/v1  → MATCH
feature/v1  → NO MATCH
```

同时测试：

```text
同一个 Commit 不重复执行
同一个 Tag 不重复执行
新 Branch 只执行一次
```

---

# 95. 版本控制

项目本身使用 Git。

推荐：

```text
main
dev
```

开发流程：

```text
功能开发
 ↓
dev
 ↓
测试
 ↓
合并
 ↓
main
 ↓
tag
```

---

# 96. 开发原则

Agent 开发时必须遵守：

### 原则 1

先实现能工作的最小功能。

### 原则 2

不要提前实现未来功能。

### 原则 3

不要为了“架构优雅”引入不必要的服务。

### 原则 4

不要引入 Redis/PostgreSQL/Kafka 等第一阶段不必要组件。

### 原则 5

不要实现 Git Server。

### 原则 6

所有外部命令调用统一经过 CommandExecutor。

### 原则 7

所有 Pipeline 都必须可追踪。

### 原则 8

所有生产部署都必须可审计。

---

# 97. Agent 开发执行方式

Agent 不要求一次完成整个项目。

采用：

```text
需求
 ↓
实现
 ↓
编译
 ↓
测试
 ↓
运行
 ↓
验证
 ↓
下一阶段
```

每一个阶段必须保证：

```text
cargo check
cargo test
cargo build
```

通过。

---

# 98. 推荐开发顺序

## Phase 1：基础框架

```text
Rust
Axum
SQLite
登录
Project
```

## Phase 2：Command Executor

实现：

```text
执行命令
实时日志
超时
取消
exit code
```

这是核心基础设施。

## Phase 3：Git

实现：

```text
Git Credential
git fetch
checkout
commit 信息
```

## Phase 4：Pipeline

实现：

```text
Clone
Runtime
Install
Build
Test
Package
```

## Phase 5：Artifact

实现：

```text
保存
查询
下载
删除
```

## Phase 6：Server

实现：

```text
SSH
SFTP
Server Test
```

## Phase 7：Deploy

实现：

```text
Backup
Upload
Deploy
Restart
Verify
```

## Phase 8：Trigger

实现：

```text
Manual
Branch
Tag
Branch Created
Tag Created
Polling
Schedule
```

## Phase 9：Environment

实现：

```text
DEV
TEST
UAT
PROD
```

## Phase 10：完善

实现：

```text
Secret
Audit
Rollback
Approval
```

---

# 99. 第一条完整验收链

项目完成第一个可用版本后，必须能够实现以下操作：

1. 浏览器访问 LiteCI。
2. 登录。
3. 创建项目 `jzx-website`。
4. 输入 Gitee Git 地址。
5. 配置 Node.js 22。
6. 配置 Install：

```bash
pnpm install --frozen-lockfile
```

7. 配置 Build：

```bash
pnpm build
```

8. 配置 Artifact：

```text
dist/
```

9. 关闭 Test。
10. 关闭 Docker Build。
11. 添加 TEST 服务器。
12. 一键测试 SSH。
13. 配置 TEST 部署目录。
14. 点击“立即执行”。
15. 系统拉取 Gitee。
16. 执行 Install。
17. 执行 Build。
18. 打包 Artifact。
19. SFTP 上传。
20. 执行部署。
21. 重启服务。
22. Health Check。
23. 页面显示 SUCCESS。
24. 可以查看完整日志。
25. 可以查看 Artifact。
26. 可以重新部署该 Artifact。

完成以上流程，V0.1 即具备实际使用价值。

---

# 100. 最终目标

LiteCI 最终不是 GitLab 的完整替代品。

它的目标是：

```text
                  Git Repository
                       │
                       ▼
                ┌───────────────┐
                │  LiteCI   │
                ├───────────────┤
                │ Trigger       │
                │ Pipeline      │
                │ Runtime       │
                │ Artifact      │
                │ Deployment    │
                │ Environment   │
                │ Server        │
                │ Log           │
                └───────┬───────┘
                        │
              ┌─────────┼─────────┐
              ▼         ▼         ▼
             DEV       TEST      UAT
                                  │
                                  ▼
                                 PROD
```

最终形成一个：

> **单机、轻量、低资源占用、Web 管理、可自建、以实际部署需求为核心的 Rust DevOps 平台。**

最重要的产品原则：

> **不追求功能数量，而追求每一个实际需要的功能都简单、可靠、可维护。**
