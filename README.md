# OpenCovibe Web Server

OpenCovibe-Web 是基于[OpenCovibe](https://github.com/AnyiWang/OpenCovibe "OpenCovibe")项目衍生而来，目标是服务器端（linux、飞牛及其它nas）的纯web coding，项目只运行一个原生 Linux Web 服务，不包含桌面客户端、Tauri、WebView、Docker，也不会安装或更新任何 Agent CLI。因此固取名为 OpenCovibe-Web 

本项目github开源地址为：[OpenCovibe-web](https://github.com/ZoomBili/OpenCovibe-web "OpenCovibe-web")

## 计划中的功能清单
- 增加“继续”对话的快捷按钮
- 支持快速下拉配置（回车键 或 ctrl+回车）发送对话
- 支持对话结束后发送企微消息、浏览器铃声
- 飞牛应用


## 一键安装

要求：

- 使用 systemd 的 Linux，支持 `amd64` 和 `arm64`（例如linux服务器或飞牛nas及其它nas设备）
- 已安装并登录 Claude Code 或 Codex，至少一个可用

从 GitHub Release 安装最新版本：

```bash
REPO="ZoomBili/OpenCovibe-web"
curl -fsSL "https://raw.githubusercontent.com/${REPO}/main/scripts/install-server.sh" \
  | sudo env OPENCOVIBE_REPO="$REPO" bash -s -- \
      --workspace /vol1/1000/aicoding（替换为你的工作空间）
```

安装器会完成以下操作：

- 下载并校验对应架构的 OpenCovibe 单二进制
- 查找当前 Linux 用户已有的 `claude` 和 `codex`
- 生成访问令牌和 `/etc/opencovibe.env`
- 以拥有 CLI 登录信息的用户创建并启动 systemd 服务
- 等待 `GET /health` 检查通过后输出访问地址和令牌

安装器不会下载、登录、升级或删除 Claude Code/Codex，也不会修改 `~/.claude` 和 `~/.codex`。

默认访问地址为 `http://服务器IP:9476`。服务默认使用执行 `sudo` 前的用户，并把上面配置的workspace作为工作目录。

## 安装过程示例（注意其中的Token，后面登录需要用到）
```
root@zh:/# curl -fsSL "https://raw.githubusercontent.com/${REPO}/main/scripts/install-server.sh" \
  | sudo env OPENCOVIBE_REPO="$REPO" bash -s -- \
      --workspace /vol1/1000/aicoding
[OpenCovibe] Using existing Claude Code: /vol1/1000/nodejs24/npm-global/bin/claude
[OpenCovibe] Using existing Codex: /vol1/1000/nodejs24/npm-global/bin/codex
[OpenCovibe] Downloading OpenCovibe v0.2.7 for linux/amd64
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
  0     0    0     0    0     0      0      0 --:--:--  0:00:01 --:--:--     0
100 4967k  100 4967k    0     0   995k      0  0:00:04  0:00:04 --:--:-- 1456k
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
  0     0    0     0    0     0      0      0 --:--:--  0:00:01 --:--:--     0
100   110  100   110    0     0     22      0  0:00:05  0:00:04  0:00:01    32
opencovibe-server_v0.2.7_linux_amd64.tar.gz: OK
[OpenCovibe] Waiting for http://127.0.0.1:9476/health
[OpenCovibe] Installed OpenCovibe v0.2.7
[OpenCovibe] URL: http://192.168.31.2:9476
[OpenCovibe] Token: e17d0abcc1eb3769c0e12c813184b8f90013227ee058a753
[OpenCovibe] Service user: root
[OpenCovibe] Workspace: /vol1/1000/aicoding
[OpenCovibe] No Claude Code or Codex files were installed or modified.
```
1、内网访问http://宿主机ip:9476或公网访问反代后的地址
<img width="2564" height="1458" alt="image" src="https://github.com/user-attachments/assets/81b1f0ba-1f9a-41f9-8ec2-496af774bf05" />

2、找到上面安装脚本完成后的持久化Token，输入Token后访问（首次）。
<img width="2666" height="1353" alt="image" src="https://github.com/user-attachments/assets/c65f4498-526d-4d68-93bb-15b2dcfebead" />

3、选择使用claudecode还是codex，自动检测后进入coding界面
<img width="2666" height="1353" alt="image" src="https://github.com/user-attachments/assets/6bd2d82c-0d24-48ef-b78f-784d61ff3d3c" />

4、服务重启命令
systemctl restart opencovibe

## 下面的为说明补充，可看可不看
## 安装选项

指定版本、运行用户和工作目录：

```bash
curl -fsSL "https://raw.githubusercontent.com/${REPO}/main/scripts/install-server.sh" \
  | sudo env OPENCOVIBE_REPO="$REPO" bash -s -- \
      --version v0.2.7 \
      --workspace /vol1/1000/aicoding
```

当 CLI 由 NVM 等工具安装且无法自动发现时，显式指定绝对路径：

```bash
curl -fsSL "https://raw.githubusercontent.com/${REPO}/main/scripts/install-server.sh" \
  | sudo env OPENCOVIBE_REPO="$REPO" bash -s -- \
      --claude-path /home/alice/.nvm/versions/node/v22.0.0/bin/claude \
      --codex-path /home/alice/.nvm/versions/node/v22.0.0/bin/codex
```

常用选项：

| 参数                | 默认值                  | 说明                              |
| ------------------- | ----------------------- | --------------------------------- |
| `--version`         | `latest`                | GitHub Release 标签               |
| `--user`            | 当前用户或 `$SUDO_USER` | 运行服务并持有 CLI 登录信息的用户 |
| `--workspace`       | 用户主目录              | 默认工作目录                      |
| `--bind`            | `0.0.0.0`               | 监听地址                          |
| `--port`            | `9476`                  | Web 端口                          |
| `--token`           | 首次安装自动生成        | 浏览器登录令牌                    |
| `--allowed-origins` | 空                      | 反向代理 Origin 白名单，逗号分隔  |
| `--no-start`        | 关闭                    | 只安装文件，不启动服务            |

完整参数使用 `bash scripts/install-server.sh --help` 查看。

## 运维

查看服务状态和实时日志：

```bash
sudo systemctl status opencovibe
sudo journalctl -u opencovibe -f
```

查看配置和登录令牌：

```bash
sudo cat /etc/opencovibe.env
```

升级时重新执行一键安装命令。安装器会下载最新 Release、替换二进制并重启服务；未显式传入 `--token` 时保留现有令牌。

卸载服务和二进制，保留 OpenCovibe 配置与数据：

```bash
curl -fsSL "https://raw.githubusercontent.com/${REPO}/main/scripts/install-server.sh" \
  | sudo env OPENCOVIBE_REPO="$REPO" bash -s -- --uninstall
```

同时删除 `/etc/opencovibe.env` 和运行用户的 `~/.opencovibe`：

```bash
curl -fsSL "https://raw.githubusercontent.com/${REPO}/main/scripts/install-server.sh" \
  | sudo env OPENCOVIBE_REPO="$REPO" bash -s -- --uninstall --purge
```

`--purge` 也不会删除 `~/.claude` 或 `~/.codex`。

## 不使用 GitHub Actions 发布

如果仓库无法使用 GitHub Actions，可在 Windows 开发机安装 Rust、Node.js 和 Zig 0.14+，登录 Git Credential Manager 后直接发布：

```powershell
git credential-manager github login --browser
.\scripts\publish-release.ps1
```

脚本会在本机交叉构建 Linux `amd64`、`arm64`，生成 SHA256，并创建或更新与 `package.json` 版本对应的 GitHub Release。令牌从 Git Credential Manager 读取，不会写入项目文件或日志。

## HTTPS 反向代理

公网部署建议让 Nginx、Caddy 或 Traefik 终止 HTTPS，只把 `9476` 暴露给本机或可信网络。代理必须转发 WebSocket 的 `Upgrade` 和 `Connection` 请求头。

使用 `https://code.example.com` 访问时可重新运行安装器并添加：

```bash
--bind 127.0.0.1 --allowed-origins https://code.example.com
```

## 手动构建

要求 Node.js 20+、Rust stable，以及已有的 Claude Code 或 Codex：

```bash
npm ci --ignore-scripts
npm run build
cargo build --release --locked --manifest-path src-tauri/Cargo.toml
sudo install -m 0755 src-tauri/target/release/opencovibe-server /usr/local/bin/
```

服务环境变量：

| 变量                         | 默认值                        | 说明                                  |
| ---------------------------- | ----------------------------- | ------------------------------------- |
| `OPENCOVIBE_BIND`            | `0.0.0.0`                     | `127.0.0.1`、`0.0.0.0`、`::1` 或 `::` |
| `OPENCOVIBE_PORT`            | `9476`                        | 端口，必须在 1024 到 65535 之间       |
| `OPENCOVIBE_TOKEN`           | 临时生成                      | 浏览器访问令牌，生产环境应固定配置    |
| `OPENCOVIBE_ALLOWED_ORIGINS` | 空                            | 反向代理 Origin 白名单                |
| `OPENCOVIBE_CLAUDE_PATH`     | 自动查找                      | Claude Code 绝对路径                  |
| `OPENCOVIBE_CODEX_PATH`      | 自动查找                      | Codex 绝对路径                        |
| `RUST_LOG`                   | `opencovibe_server=info,warn` | 日志过滤器                            |

手动 systemd 配置参考 `deploy/opencovibe.service` 和 `deploy/opencovibe.env.example`。一键安装脚本会按实际用户动态生成这两个文件。

## 开发验证

```bash
npm run lint
npm run format:check
npm run check
npm test
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --locked --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --locked --manifest-path src-tauri/Cargo.toml
```
