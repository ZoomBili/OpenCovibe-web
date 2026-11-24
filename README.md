# OpenCovibe Web Server 

OpenCovibe-Web 是基于[OpenCovibe](https://github.com/AnyiWang/OpenCovibe "OpenCovibe")项目衍生而来，目标是服务器端（linux、飞牛及其它nas）的纯web coding，项目只运行一个原生 Linux Web 服务，不包含桌面客户端、Tauri、WebView、Docker，也不会安装或更新任何 Agent CLI。因此固取名为 OpenCovibe-Web 

本项目github开源地址为：[OpenCovibe-web](https://github.com/ZoomBili/OpenCovibe-web "OpenCovibe-web")

现在同时支持桌面和手机浏览器访问，手机端会自动切换为紧凑布局与抽屉式导航。

## 261124更新：通知推送（v0.2.9）

新增「设置 → 通知」标签页。任务完成、运行失败或等待审批时，可以把结果推送到手机：

- **触发事件**：任务完成、运行失败、等待工具审批，三类可分别开关；总开关默认关闭，未启用时不会产生任何外发请求。只对实时事件生效，导入的 CLI 历史记录不会触发通知。
- **企业微信**：填写群机器人 Webhook 即可发送文本消息，可选 @ 指定手机号；超过企业微信 2048 字节上限的消息会按 UTF-8 边界截断。
- **自定义 HTTP**：支持 `GET` / `POST` / `PUT` / `PATCH`、自定义请求头和请求体模板，可对接 Bark、ntfy、自建网关等。模板支持 `{event}`、`{title}`、`{status}`、`{status_label}`、`{run_id}`、`{run_name}`、`{session}`、`{turn}`、`{turn_preview}`、`{project}`、`{cwd}`、`{agent}`、`{message}`、`{duration}`、`{tokens_m}`、`{run_url}`、`{time}`、`{text}`、`{text_body}` 占位符；JSON 模板里的占位符必须放在引号内（如 `"title": "{title}"`），保存或试发前会本地校验，避免把非法 JSON 发给端点。
- **消息内容**：包含工程、会话、当前轮次与输入预览、状态、耗时和 token 用量；可设置通知语言（中 / 英）、项目名显示为目录名还是完整路径，以及用于生成 `{run_url}` 的会话链接根地址。
- **试发**：每个通道都能在保存前用当前表单内容发送测试消息，无需先保存。
- **可靠性**：通知在独立 tokio 任务中异步发送，超时 10 秒；发送失败只写日志，不会阻塞 Agent 会话或影响聊天。

同时包含以下修复：

- 侧栏版本号改为读取 `package.json`，与发布包 / FPK 版本保持一致（当前 `v0.2.9`）。
- 修复 WebSocket 重连卡死：连接被拒绝时 `connect()` 的 Promise 不会结束，导致界面静默冻结；现在所有终止路径都会结束等待并按退避策略重连。
- 修复插件市场清单 `lspServers` 字段大小写不一致导致解析不到的问题。

## 261124更新：移动端适配

本次更新重点补齐手机 / 平板上的使用体验，桌面端行为保持不变：

- **响应式布局**：`< 1024px`（Tailwind `lg`）进入移动布局，`< 640px`（`sm`）为窄屏档。新增 `src/lib/utils/viewport.svelte.ts` 单例，向组件暴露 `viewport.mobile`、`viewport.narrow`、`viewport.touch` 三个响应式状态。
- **抽屉式导航**：左侧项目 / 会话栏和聊天右侧的工具活动面板，在手机上改为带遮罩的滑出抽屉；点击遮罩、按 Esc、切换页面或选中项目后自动关闭。桌面端仍保留可拖拽宽度的内联栏。
- **刘海屏与软键盘**：`viewport-fit=cover` 配合 `env(safe-area-inset-*)`（`.pt-safe`、`.pb-safe`、`.pb-composer`），高度改用 `100dvh`；软键盘弹出时压缩布局而不是遮挡输入框；移动端表单控件强制 16px 字号，避免 iOS 聚焦时页面缩放。
- **触屏交互**：新增 Tailwind `touch:` / `mouse:` 变体，原本“悬停才显示”的操作在触屏设备上默认可见；手机上的按钮触控区增大到 36px；手机上 Enter 换行、由发送按钮发送（外接键盘或桌面仍为 Enter 发送）。
- **输入与弹层**：模型 / 权限下拉、附件面板、斜杠菜单等按视口宽度夹紧（clamp），不再溢出屏幕；手机上进入会话时不自动聚焦输入框，避免软键盘遮挡对话。
- **页面细节**：手机状态栏隐藏次要文字、保留图标，并显示上下文占用百分比；历史、设置、用量、插件、发布说明等页面改为窄屏单列或双列布局。
- **服务端修复**：
  - root 运行时 Claude Code 不支持 `bypassPermissions`，启动权限改为回退到 `auto`，并修复了等待 `session_init` 才派发首轮消息导致界面卡在“正在启动会话”的死锁。
  - 过滤 `[claude-code:unrecognized_model]` 这类自定义网关模型的无害 stderr 提示，不再混淆聊天记录。

## 计划中的功能清单
- 支持对话结束后浏览器铃声 / 桌面通知
- ~~支持对话结束后发送企微消息~~（v0.2.9 已支持，见上方通知推送）
- ~~飞牛应用~~（已提供 FPK，见 `fnos/`）


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
[OpenCovibe] Downloading OpenCovibe v0.2.9 for linux/amd64
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
  0     0    0     0    0     0      0      0 --:--:--  0:00:01 --:--:--     0
100 4967k  100 4967k    0     0   995k      0  0:00:04  0:00:04 --:--:-- 1456k
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
  0     0    0     0    0     0      0      0 --:--:--  0:00:01 --:--:--     0
100   110  100   110    0     0     22      0  0:00:05  0:00:04  0:00:01    32
opencovibe-server_v0.2.9_linux_amd64.tar.gz: OK
[OpenCovibe] Waiting for http://127.0.0.1:9476/health
[OpenCovibe] Installed OpenCovibe v0.2.9
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
      --version v0.2.9 \
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
