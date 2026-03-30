# XTG Web 服务部署（Ubuntu Server 24.04 LTS）

## 构建

在开发机或 CI 上：

1. 安装 Rust、`nodejs` / `npm`。
2. 编译前端并输出到 `crates/xtg-server/static`：

```bash
cd frontend && npm ci && npm run build
```

3. 编译服务器二进制：

```bash
cargo build --release -p xtg-server
```

产物：`target/release/xtg-server`。将 `static` 目录（即 `crates/xtg-server/static/` 下构建产物）与二进制同目录部署，或设置 `XTG_STATIC_DIR` 指向该目录。

## 环境变量

| 变量 | 说明 |
|------|------|
| `XTG_LISTEN` | 监听地址，默认 `0.0.0.0:8080` |
| `XTG_DATA_DIR` | 数据目录（配置、session、游标等），默认 XDG 数据目录下 `xtg/xtg-app` |
| `XTG_STATIC_DIR` | 前端静态文件目录；默认若存在 `crates/xtg-server/static/index.html` 则用该路径，否则 `./static` |
| `XTG_ADMIN_TOKEN` | 若设置，则所有 `/api/*`（除 `/api/health`）需在请求头 `Authorization: Bearer <token>` |
| `RUST_LOG` | 如 `info`、`xtg_server=debug` |

## systemd 示例

`/etc/systemd/system/xtg.service`：

```ini
[Unit]
Description=XTG Web
After=network.target

[Service]
Type=simple
User=xtg
WorkingDirectory=/opt/xtg
Environment=XTG_LISTEN=127.0.0.1:8080
Environment=XTG_DATA_DIR=/var/lib/xtg
Environment=XTG_STATIC_DIR=/opt/xtg/static
Environment=XTG_ADMIN_TOKEN=请替换为强随机串
Environment=RUST_LOG=info
ExecStart=/opt/xtg/xtg-server
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

```bash
sudo mkdir -p /opt/xtg /var/lib/xtg
sudo chown xtg:xtg /var/lib/xtg
sudo systemctl daemon-reload
sudo systemctl enable --now xtg
```

## 更新（与上文 systemd / 路径一致）

以下假定你已按本文档使用 **`/opt/xtg`**（程序 + `static`）、**`/var/lib/xtg`**（`XTG_DATA_DIR`）、**`systemctl` 管理 `xtg`**、**nginx 反代到 `127.0.0.1:8080`**。更新时**不要删除或覆盖** `/var/lib/xtg`（配置、TG session、游标等）。

### 在服务器上直接拉代码并编译

```bash
cd /path/to/推特帖子追踪BOT   # 你的仓库目录
git pull
cd frontend && npm ci && npm run build
cd .. && cargo build --release -p xtg-server
sudo install -m 755 target/release/xtg-server /opt/xtg/xtg-server
sudo rsync -a --delete crates/xtg-server/static/ /opt/xtg/static/
sudo systemctl restart xtg
```

`rsync --delete` 会移除旧版带 hash 的 JS/CSS，避免前端与后端版本不一致。

### 仅本机改好再同步到服务器

若仓库在开发机，可在开发机 `git push` 后，在服务器执行上一段；或在开发机打包后上传：

- 将 **`xtg-server`（Linux 下编译的 `target/release/xtg-server`）** 覆盖到 `/opt/xtg/xtg-server`
- 将 **`crates/xtg-server/static/`** 整目录同步到 **`/opt/xtg/static/`**（同样建议 `rsync -a --delete`）

**注意**：在 Windows 上 `cargo build` 生成的是 `xtg-server.exe`，不能替换 Linux 上的 `/opt/xtg/xtg-server`。应在 **Linux 环境**（服务器本机或 WSL/CI）编译 `xtg-server`，或把 Linux 产物上传。

### 用 zip 上传（无 git、或习惯打包再传）

**方式一：zip 里是「可直接部署的两样」**（推荐）：在能编出 **Linux 版** `xtg-server` 的环境（服务器本机、WSL Ubuntu、CI）里先执行上文「构建」三步，再只打小包：

- `xtg-server`（单文件二进制）
- `static/`（即 `crates/xtg-server/static/` 整个目录，内含 `index.html` 与 `assets/`）

上传到服务器后（示例路径 `/tmp/xtg-update.zip`）：

```bash
sudo systemctl stop xtg
unzip -o /tmp/xtg-update.zip -d /tmp/xtg-update
sudo install -m 755 /tmp/xtg-update/xtg-server /opt/xtg/xtg-server
sudo rm -rf /opt/xtg/static/*
sudo cp -a /tmp/xtg-update/static/. /opt/xtg/static/
sudo chown -R root:root /opt/xtg/xtg-server /opt/xtg/static
sudo systemctl start xtg
```

若 zip 解压后目录结构是「根目录下直接是 `index.html` 与 `assets/`」，则把 `sudo cp` 源改为该目录，例如 `sudo cp -a /tmp/xtg-update/. /opt/xtg/static/`（确保 `/opt/xtg/static/index.html` 存在）。

**方式二：zip 里是整份源码**：上传解压到某目录后，在服务器上执行 `cd frontend && npm ci && npm run build`、`cargo build --release -p xtg-server`，再按上文 `install` + `rsync static` 部署。**不要**把 `node_modules` 或 `target/` 从 Windows 打进 zip 再解压到 Linux 上凑合用（架构不一致）；源码 zip 在 Linux 上重新 `npm ci` / `cargo build` 即可。

两种方式均**不要**解压覆盖 **`/var/lib/xtg`**。

### 变更后检查

```bash
sudo systemctl status xtg
journalctl -u xtg -n 50 --no-pager
```

nginx 与证书、域名未改时**无需** `nginx -t` / `reload`。若仅改了 systemd 环境变量或单元文件，需：`sudo systemctl daemon-reload && sudo systemctl restart xtg`。

## nginx 反向代理（HTTPS）

仅对外暴露 80/443，将管理台与 API 反代到本机 `127.0.0.1:8080`：

```nginx
server {
    listen 443 ssl http2;
    server_name xtg.example.com;
    ssl_certificate     /etc/letsencrypt/live/xtg.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/xtg.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_buffering off;
    }
}
```

`proxy_buffering off` 有利于 SSE 日志流。

## 安全说明

- 公网务必启用 HTTPS + 强 `XTG_ADMIN_TOKEN`；配置与 TG session 含敏感信息。
- 若未设置 `XTG_ADMIN_TOKEN`，服务端仅记录警告，请依赖内网或 nginx 访问控制。

## 本地开发

终端 1：`cargo run -p xtg-server`（仓库根目录，便于默认找到 `crates/xtg-server/static`）。

终端 2：`cd frontend && npm run dev`（Vite 将 `/api` 代理到 `http://127.0.0.1:8080`）。
