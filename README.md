# OpenCovibe Web Server

OpenCovibe Web Server is a browser UI for Claude Code and Codex. It runs as one native Linux service with no desktop client, Tauri, WebView, Docker, or bundled Agent CLI.

[简体中文](README.zh-CN.md)

## One-Command Install

Requirements:

- A systemd-based Linux server on `amd64` or `arm64`
- Claude Code or Codex already installed and authenticated
- `curl`, `tar`, `sha256sum`, and `sudo`

Install the latest GitHub Release:

```bash
REPO="ZoomBili/OpenCovibe-web"
curl -fsSL "https://raw.githubusercontent.com/${REPO}/master/scripts/install-server.sh" \
  | sudo env OPENCOVIBE_REPO="$REPO" bash
```

The installer downloads and verifies one OpenCovibe binary, discovers the current user's existing Claude/Codex executables, creates `/etc/opencovibe.env`, installs a systemd unit under that same user, and waits for `GET /health` to pass.

It never installs, updates, authenticates, or removes Claude Code or Codex. It does not modify `~/.claude` or `~/.codex`.

Open `http://SERVER_IP:9476` and sign in with the token printed by the installer. The default service user is the user from before `sudo`, and the default workspace is that user's home directory.

## Install Options

Pin a release and select the service user and workspace:

```bash
curl -fsSL "https://raw.githubusercontent.com/${REPO}/master/scripts/install-server.sh" \
  | sudo env OPENCOVIBE_REPO="$REPO" bash -s -- \
      --version v0.2.6 \
      --user alice \
      --workspace /home/alice/projects
```

For NVM or another shell-managed installation, pass explicit executable paths if auto-detection cannot find them:

```bash
--claude-path /home/alice/.nvm/versions/node/v22.0.0/bin/claude
--codex-path /home/alice/.nvm/versions/node/v22.0.0/bin/codex
```

Run `bash scripts/install-server.sh --help` for all options.

## Operations

```bash
sudo systemctl status opencovibe
sudo journalctl -u opencovibe -f
sudo cat /etc/opencovibe.env
```

Run the install command again to upgrade. The existing browser token is retained unless `--token` is supplied.

Uninstall while retaining OpenCovibe configuration and data:

```bash
curl -fsSL "https://raw.githubusercontent.com/${REPO}/master/scripts/install-server.sh" \
  | sudo env OPENCOVIBE_REPO="$REPO" bash -s -- --uninstall
```

Add `--purge` to also remove `/etc/opencovibe.env` and the service user's `~/.opencovibe`. Claude/Codex data is never removed.

## Reverse Proxy

Terminate HTTPS with Nginx, Caddy, or Traefik and proxy WebSocket `Upgrade` and `Connection` headers. For `https://code.example.com`, install with:

```bash
--bind 127.0.0.1 --allowed-origins https://code.example.com
```

Avoid exposing the plain HTTP port directly to the public internet.

## Build From Source

Requires Node.js 20+, stable Rust, and an existing Claude Code or Codex installation:

```bash
npm ci --ignore-scripts
npm run build
cargo build --release --locked --manifest-path src-tauri/Cargo.toml
sudo install -m 0755 src-tauri/target/release/opencovibe-server /usr/local/bin/
```

| Variable                     | Default                       | Description                                |
| ---------------------------- | ----------------------------- | ------------------------------------------ |
| `OPENCOVIBE_BIND`            | `0.0.0.0`                     | `127.0.0.1`, `0.0.0.0`, `::1`, or `::`     |
| `OPENCOVIBE_PORT`            | `9476`                        | Port from 1024 through 65535               |
| `OPENCOVIBE_TOKEN`           | generated                     | Browser access token; set it in production |
| `OPENCOVIBE_ALLOWED_ORIGINS` | empty                         | Comma-separated reverse-proxy origins      |
| `OPENCOVIBE_CLAUDE_PATH`     | auto                          | Absolute Claude Code executable path       |
| `OPENCOVIBE_CODEX_PATH`      | auto                          | Absolute Codex executable path             |
| `RUST_LOG`                   | `opencovibe_server=info,warn` | Rust log filter                            |

See `deploy/opencovibe.service` and `deploy/opencovibe.env.example` for manual systemd deployment. The installer generates these files using the actual Linux user and paths.
