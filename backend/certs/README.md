# TLS 证书目录

后端首次启动时若 `tls_cert`/`tls_key` 文件不存在，会调用 `rcgen` 自签生成占位证书并落盘到本目录。

> 自签证书仅适用于 dev / 测试。生产请用受信任 CA 签发的正式证书替换。

## 文件说明

- `server.crt` — 服务端证书（PEM）
- `server.key` — 服务端私钥（PEM）
- `ca.crt` — 客户端 CA（mTLS 启用时校验客户端证书）

## 不提交真实证书

`../.gitignore` 已忽略 `certs/*.crt|key|pem`，仅保留 `.gitkeep`。

## 重新生成

删除上述文件后重启后端即可自动重新自签。
