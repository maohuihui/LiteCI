# 凭证和 Git 同步

项目的 `git_auth_id` 可关联 `credentials.id`。同步接口会根据仓库形式读取对应的加密凭证：

- HTTPS URL 使用 `https_token` 凭证
- `git@host:path` URL 使用 `ssh_key` 凭证

凭证只在同步执行期间解密，Git 使用临时 askpass 或临时私钥文件，执行结束后自动清理。

测试构造函数可以使用固定测试密钥；生产入口必须配置 `LITECI_CREDENTIAL_KEY`（64 位十六进制字符串）。
