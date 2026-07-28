#!/usr/bin/env bash
set -Eeuo pipefail

REPO="${OPENCOVIBE_REPO:-ZoomBili/OpenCovibe-web}"
VERSION="${OPENCOVIBE_VERSION:-latest}"
PREFIX="${OPENCOVIBE_PREFIX:-/usr/local}"
RUN_USER="${OPENCOVIBE_USER:-}"
WORKSPACE="${OPENCOVIBE_WORKSPACE:-}"
BIND="${OPENCOVIBE_BIND:-}"
PORT="${OPENCOVIBE_PORT:-}"
TOKEN="${OPENCOVIBE_TOKEN:-}"
ALLOWED_ORIGINS="${OPENCOVIBE_ALLOWED_ORIGINS:-}"
CLAUDE_PATH="${OPENCOVIBE_CLAUDE_PATH:-}"
CODEX_PATH="${OPENCOVIBE_CODEX_PATH:-}"
NO_START=false
UNINSTALL=false
PURGE=false
TOKEN_EXPLICIT=false
RUN_USER_EXPLICIT=false
WORKSPACE_EXPLICIT=false
BIND_EXPLICIT=false
PORT_EXPLICIT=false
ORIGINS_EXPLICIT=false
CLAUDE_PATH_EXPLICIT=false
CODEX_PATH_EXPLICIT=false

[[ -z "${OPENCOVIBE_USER+x}" ]] || RUN_USER_EXPLICIT=true
[[ -z "${OPENCOVIBE_WORKSPACE+x}" ]] || WORKSPACE_EXPLICIT=true
[[ -z "${OPENCOVIBE_BIND+x}" ]] || BIND_EXPLICIT=true
[[ -z "${OPENCOVIBE_PORT+x}" ]] || PORT_EXPLICIT=true
[[ -z "${OPENCOVIBE_TOKEN+x}" ]] || TOKEN_EXPLICIT=true
[[ -z "${OPENCOVIBE_ALLOWED_ORIGINS+x}" ]] || ORIGINS_EXPLICIT=true
[[ -z "${OPENCOVIBE_CLAUDE_PATH+x}" ]] || CLAUDE_PATH_EXPLICIT=true
[[ -z "${OPENCOVIBE_CODEX_PATH+x}" ]] || CODEX_PATH_EXPLICIT=true

SERVICE_NAME="opencovibe"
SERVICE_FILE="/etc/systemd/system/${SERVICE_NAME}.service"
ENV_FILE="/etc/opencovibe.env"
TMP_DIR=""

log() {
  printf '[OpenCovibe] %s\n' "$*"
}

die() {
  printf '[OpenCovibe] ERROR: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Install OpenCovibe as a native Linux systemd service.

Usage:
  install-server.sh [options]
  install-server.sh --uninstall [--purge] [--prefix PATH]

Options:
  --repo OWNER/REPO          GitHub repository containing releases
  --version VERSION          Release tag, for example v0.2.6 (default: latest)
  --prefix PATH              Installation prefix (default: /usr/local)
  --user USER                Linux user that already has Claude/Codex installed
  --workspace PATH           Default working directory (default: user's home)
  --bind ADDRESS             Listen address (default: 0.0.0.0)
  --port PORT                Listen port (default: 9476)
  --token TOKEN              Browser login token (generated on first install)
  --allowed-origins LIST     Comma-separated HTTPS reverse-proxy origins
  --claude-path PATH         Existing Claude Code executable
  --codex-path PATH          Existing Codex executable
  --no-start                 Install files without enabling or starting systemd
  --uninstall                Remove the binary and systemd unit
  --purge                    Also remove OpenCovibe config and user data
  -h, --help                 Show this help

This installer never installs, upgrades, or removes Claude Code or Codex.
EOF
}

while (($# > 0)); do
  case "$1" in
    --repo)
      (($# >= 2)) || die "--repo requires a value"
      REPO="$2"
      shift 2
      ;;
    --version)
      (($# >= 2)) || die "--version requires a value"
      VERSION="$2"
      shift 2
      ;;
    --prefix)
      (($# >= 2)) || die "--prefix requires a value"
      PREFIX="${2%/}"
      shift 2
      ;;
    --user)
      (($# >= 2)) || die "--user requires a value"
      RUN_USER="$2"
      RUN_USER_EXPLICIT=true
      shift 2
      ;;
    --workspace)
      (($# >= 2)) || die "--workspace requires a value"
      WORKSPACE="$2"
      WORKSPACE_EXPLICIT=true
      shift 2
      ;;
    --bind)
      (($# >= 2)) || die "--bind requires a value"
      BIND="$2"
      BIND_EXPLICIT=true
      shift 2
      ;;
    --port)
      (($# >= 2)) || die "--port requires a value"
      PORT="$2"
      PORT_EXPLICIT=true
      shift 2
      ;;
    --token)
      (($# >= 2)) || die "--token requires a value"
      TOKEN="$2"
      TOKEN_EXPLICIT=true
      shift 2
      ;;
    --allowed-origins)
      (($# >= 2)) || die "--allowed-origins requires a value"
      ALLOWED_ORIGINS="$2"
      ORIGINS_EXPLICIT=true
      shift 2
      ;;
    --claude-path)
      (($# >= 2)) || die "--claude-path requires a value"
      CLAUDE_PATH="$2"
      CLAUDE_PATH_EXPLICIT=true
      shift 2
      ;;
    --codex-path)
      (($# >= 2)) || die "--codex-path requires a value"
      CODEX_PATH="$2"
      CODEX_PATH_EXPLICIT=true
      shift 2
      ;;
    --no-start)
      NO_START=true
      shift
      ;;
    --uninstall)
      UNINSTALL=true
      shift
      ;;
    --purge)
      PURGE=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

as_root() {
  if [[ "$(id -u)" -eq 0 ]]; then
    "$@"
  else
    command -v sudo >/dev/null 2>&1 || die "sudo is required for system installation"
    sudo "$@"
  fi
}

read_root_file() {
  local path="$1"
  if [[ "$(id -u)" -eq 0 ]]; then
    cat "$path"
  else
    sudo cat "$path"
  fi
}

read_env_value() {
  local key="$1"
  local path="$2"
  local value
  value="$(read_root_file "$path" 2>/dev/null | awk -F= -v key="$key" '$1 == key { sub(/^[^=]*=/, ""); print; exit }' || true)"
  if [[ "$value" == \"*\" && "$value" == *\" ]]; then
    value="${value:1:${#value}-2}"
  fi
  printf '%s' "$value"
}

cleanup() {
  if [[ -n "$TMP_DIR" && -d "$TMP_DIR" ]]; then
    rm -rf -- "$TMP_DIR"
  fi
}
trap cleanup EXIT

detect_default_user() {
  if [[ -n "$RUN_USER" ]]; then
    return
  fi
  if [[ -n "${SUDO_USER:-}" && "${SUDO_USER}" != "root" ]]; then
    RUN_USER="$SUDO_USER"
  else
    RUN_USER="$(id -un)"
  fi
}

user_login() {
  local command_text="$1"
  if [[ "$(id -un)" == "$RUN_USER" ]]; then
    HOME="$USER_HOME" bash -lc "$command_text"
  elif [[ "$(id -u)" -eq 0 ]]; then
    if command -v runuser >/dev/null 2>&1; then
      runuser -u "$RUN_USER" -- env HOME="$USER_HOME" bash -lc "$command_text"
    else
      su -s /bin/bash "$RUN_USER" -c "$command_text"
    fi
  else
    sudo -Hiu "$RUN_USER" bash -lc "$command_text"
  fi
}

resolve_cli() {
  local name="$1"
  local explicit="$2"
  local strict_explicit="$3"
  local found=""
  local candidate=""
  local quoted=""

  if [[ -n "$explicit" ]]; then
    if [[ "$explicit" != /* ]]; then
      [[ "$strict_explicit" != true ]] || die "$name path must be absolute: $explicit"
    elif [[ -x "$explicit" ]]; then
      quoted="$(printf '%q' "$explicit")"
      if user_login "test -x $quoted" >/dev/null 2>&1; then
        printf '%s' "$explicit"
        return
      fi
    fi
    [[ "$strict_explicit" != true ]] || die "$name executable is not accessible to $RUN_USER: $explicit"
  fi

  found="$(user_login "command -v $name 2>/dev/null || true" 2>/dev/null | tail -n 1)"
  if [[ "$found" = /* && -x "$found" ]]; then
    quoted="$(printf '%q' "$found")"
    if user_login "test -x $quoted" >/dev/null 2>&1; then
      printf '%s' "$found"
      return
    fi
  fi

  for candidate in \
    "$USER_HOME/.local/bin/$name" \
    "$USER_HOME/.claude/bin/$name" \
    "$USER_HOME/.claude/local/$name" \
    "$USER_HOME/.codex/bin/$name" \
    "$USER_HOME/.npm-global/bin/$name" \
    "$USER_HOME/.bun/bin/$name" \
    "/usr/local/bin/$name" \
    "/usr/bin/$name"; do
    if [[ -x "$candidate" ]]; then
      quoted="$(printf '%q' "$candidate")"
      if user_login "test -x $quoted" >/dev/null 2>&1; then
        printf '%s' "$candidate"
        return
      fi
    fi
  done

  shopt -s nullglob
  local nvm_candidates=("$USER_HOME"/.nvm/versions/node/*/bin/"$name")
  shopt -u nullglob
  if ((${#nvm_candidates[@]} > 0)); then
    candidate="$(printf '%s\n' "${nvm_candidates[@]}" | sort -V | tail -n 1)"
    if [[ -x "$candidate" ]]; then
      quoted="$(printf '%q' "$candidate")"
      if user_login "test -x $quoted" >/dev/null 2>&1; then
        printf '%s' "$candidate"
        return
      fi
    fi
  fi
}

append_path_dir() {
  local directory="$1"
  [[ -n "$directory" ]] || return
  case ":$SERVICE_PATH:" in
    *":$directory:"*) ;;
    *) SERVICE_PATH="$directory${SERVICE_PATH:+:$SERVICE_PATH}" ;;
  esac
}

systemd_escape() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  value="${value//%/%%}"
  printf '%s' "$value"
}

env_escape() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  printf '%s' "$value"
}

generate_token() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 24
  else
    od -An -N24 -tx1 /dev/urandom | tr -d ' \n'
  fi
}

uninstall_server() {
  local installed_binary="${PREFIX}/bin/opencovibe-server"
  local purge_user="$RUN_USER"
  local purge_home=""
  local unit_content=""

  if read_root_file "$SERVICE_FILE" >/dev/null 2>&1; then
    unit_content="$(read_root_file "$SERVICE_FILE")"
    installed_binary="$(printf '%s\n' "$unit_content" | awk -F= '/^ExecStart=/{ value=$2; gsub(/^"|"$/, "", value); print value; exit }')"
    if [[ -z "$purge_user" ]]; then
      purge_user="$(printf '%s\n' "$unit_content" | awk -F= '/^User=/{ print $2; exit }')"
    fi
  fi

  if command -v systemctl >/dev/null 2>&1; then
    as_root systemctl disable --now "$SERVICE_NAME" >/dev/null 2>&1 || true
  fi
  as_root rm -f -- "$SERVICE_FILE"
  [[ -n "$installed_binary" ]] && as_root rm -f -- "$installed_binary"

  if command -v systemctl >/dev/null 2>&1; then
    as_root systemctl daemon-reload
    as_root systemctl reset-failed "$SERVICE_NAME" >/dev/null 2>&1 || true
  fi

  if [[ "$PURGE" == true ]]; then
    as_root rm -f -- "$ENV_FILE"
    if [[ -n "$purge_user" ]] && id "$purge_user" >/dev/null 2>&1; then
      purge_home="$(getent passwd "$purge_user" | cut -d: -f6)"
      if [[ -n "$purge_home" && "$purge_home" != "/" ]]; then
        as_root rm -rf -- "$purge_home/.opencovibe"
      fi
    fi
    log "Removed OpenCovibe configuration and data. Claude/Codex data was not touched."
  else
    log "Kept $ENV_FILE and the user's ~/.opencovibe data."
  fi
  log "OpenCovibe has been uninstalled."
}

[[ "$(uname -s)" == "Linux" ]] || die "only Linux is supported"
[[ "$PREFIX" = /* ]] || die "--prefix must be an absolute path"
for value in "$PREFIX" "$TOKEN" "$ALLOWED_ORIGINS" "$CLAUDE_PATH" "$CODEX_PATH"; do
  [[ "$value" != *$'\n'* && "$value" != *$'\r'* ]] || die "configuration values must not contain newlines"
done

if [[ "$PURGE" == true && "$UNINSTALL" != true ]]; then
  die "--purge must be used with --uninstall"
fi

if [[ "$UNINSTALL" == true ]]; then
  uninstall_server
  exit 0
fi

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v tar >/dev/null 2>&1 || die "tar is required"
command -v sha256sum >/dev/null 2>&1 || die "sha256sum is required"
command -v systemctl >/dev/null 2>&1 || die "systemd is required"
[[ -d /run/systemd/system ]] || die "systemd is not running as PID 1"
[[ "$REPO" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || die "invalid GitHub repository: $REPO"

EXISTING_UNIT="$(read_root_file "$SERVICE_FILE" 2>/dev/null || true)"
if [[ -n "$EXISTING_UNIT" ]]; then
  if [[ "$RUN_USER_EXPLICIT" != true && -z "$RUN_USER" ]]; then
    RUN_USER="$(printf '%s\n' "$EXISTING_UNIT" | awk -F= '/^User=/{ print $2; exit }')"
  fi
  if [[ "$WORKSPACE_EXPLICIT" != true && -z "$WORKSPACE" ]]; then
    WORKSPACE="$(printf '%s\n' "$EXISTING_UNIT" | awk -F= '/^WorkingDirectory=/{ sub(/^[^=]*=/, ""); gsub(/^"|"$/, ""); print; exit }')"
  fi
fi

detect_default_user
id "$RUN_USER" >/dev/null 2>&1 || die "Linux user does not exist: $RUN_USER"
command -v getent >/dev/null 2>&1 || die "getent is required"
USER_HOME="$(getent passwd "$RUN_USER" | cut -d: -f6)"
USER_GROUP="$(id -gn "$RUN_USER")"
[[ -n "$USER_HOME" && "$USER_HOME" = /* ]] || die "cannot determine home directory for $RUN_USER"

if [[ -z "$WORKSPACE" ]]; then
  WORKSPACE="$USER_HOME"
fi
[[ "$WORKSPACE" = /* ]] || die "workspace must be an absolute path"
[[ "$WORKSPACE" != *$'\n'* ]] || die "workspace must not contain newlines"

if read_root_file "$ENV_FILE" >/dev/null 2>&1; then
  if [[ "$BIND_EXPLICIT" != true && -z "$BIND" ]]; then
    BIND="$(read_env_value OPENCOVIBE_BIND "$ENV_FILE")"
  fi
  if [[ "$PORT_EXPLICIT" != true && -z "$PORT" ]]; then
    PORT="$(read_env_value OPENCOVIBE_PORT "$ENV_FILE")"
  fi
  if [[ "$TOKEN_EXPLICIT" != true && -z "$TOKEN" ]]; then
    TOKEN="$(read_env_value OPENCOVIBE_TOKEN "$ENV_FILE")"
  fi
  if [[ "$ORIGINS_EXPLICIT" != true ]]; then
    ALLOWED_ORIGINS="$(read_env_value OPENCOVIBE_ALLOWED_ORIGINS "$ENV_FILE")"
  fi
  if [[ -z "$CLAUDE_PATH" ]]; then
    CLAUDE_PATH="$(read_env_value OPENCOVIBE_CLAUDE_PATH "$ENV_FILE")"
  fi
  if [[ -z "$CODEX_PATH" ]]; then
    CODEX_PATH="$(read_env_value OPENCOVIBE_CODEX_PATH "$ENV_FILE")"
  fi
fi

[[ -n "$BIND" ]] || BIND="0.0.0.0"
[[ -n "$PORT" ]] || PORT="9476"
[[ "$ALLOWED_ORIGINS" != *$'\n'* && "$ALLOWED_ORIGINS" != *$'\r'* ]] \
  || die "allowed origins must not contain newlines"
[[ "$PORT" =~ ^[0-9]+$ ]] || die "port must be numeric"
((PORT >= 1024 && PORT <= 65535)) || die "port must be between 1024 and 65535"
case "$BIND" in
  127.0.0.1|0.0.0.0|::1|::) ;;
  *) die "bind must be one of 127.0.0.1, 0.0.0.0, ::1, ::" ;;
esac

CLAUDE_PATH="$(resolve_cli claude "$CLAUDE_PATH" "$CLAUDE_PATH_EXPLICIT")"
CODEX_PATH="$(resolve_cli codex "$CODEX_PATH" "$CODEX_PATH_EXPLICIT")"
if [[ -z "$CLAUDE_PATH" && -z "$CODEX_PATH" ]]; then
  die "neither Claude Code nor Codex was found for user $RUN_USER; install and log in to at least one CLI first"
fi
[[ -z "$CLAUDE_PATH" ]] || log "Using existing Claude Code: $CLAUDE_PATH"
[[ -z "$CODEX_PATH" ]] || log "Using existing Codex: $CODEX_PATH"

if [[ -z "$TOKEN" ]]; then
  TOKEN="$(generate_token)"
fi
[[ ${#TOKEN} -ge 16 ]] || die "token must contain at least 16 characters"
[[ "$TOKEN" != *$'\n'* && "$TOKEN" != *$'\r'* ]] || die "token must not contain newlines"

if [[ ! -d "$WORKSPACE" ]]; then
  as_root install -d -m 0755 -o "$RUN_USER" -g "$USER_GROUP" "$WORKSPACE"
fi
[[ -d "$WORKSPACE" ]] || die "workspace is not a directory: $WORKSPACE"
WORKSPACE_SHELL="$(printf '%q' "$WORKSPACE")"
user_login "test -r $WORKSPACE_SHELL && test -w $WORKSPACE_SHELL && test -x $WORKSPACE_SHELL" \
  || die "user $RUN_USER cannot access workspace: $WORKSPACE"

case "$(uname -m)" in
  x86_64|amd64) ARCH="amd64" ;;
  aarch64|arm64) ARCH="arm64" ;;
  *) die "unsupported architecture: $(uname -m)" ;;
esac

if [[ "$VERSION" == "latest" ]]; then
  log "Resolving latest release from $REPO"
  RELEASE_URL="$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest")"
  VERSION="${RELEASE_URL##*/}"
  [[ -n "$VERSION" && "$VERSION" != "latest" ]] || die "could not resolve latest release"
elif [[ "$VERSION" != v* ]]; then
  VERSION="v$VERSION"
fi

ASSET="opencovibe-server_${VERSION}_linux_${ARCH}.tar.gz"
BASE_URL="https://github.com/$REPO/releases/download/$VERSION"
TMP_DIR="$(mktemp -d)"
log "Downloading OpenCovibe $VERSION for linux/$ARCH"
curl -fL --retry 3 --retry-delay 2 -o "$TMP_DIR/$ASSET" "$BASE_URL/$ASSET"
curl -fL --retry 3 --retry-delay 2 -o "$TMP_DIR/$ASSET.sha256" "$BASE_URL/$ASSET.sha256"
(
  cd "$TMP_DIR"
  sha256sum -c "$ASSET.sha256"
)
mkdir -p "$TMP_DIR/extract"
tar -xzf "$TMP_DIR/$ASSET" -C "$TMP_DIR/extract"
BINARY="$(find "$TMP_DIR/extract" -type f -name opencovibe-server -print -quit)"
[[ -n "$BINARY" ]] || die "release archive does not contain opencovibe-server"

as_root install -d -m 0755 "${PREFIX}/bin"
as_root install -m 0755 "$BINARY" "${PREFIX}/bin/opencovibe-server"

SERVICE_PATH="$(user_login 'printf "%s\n" "$PATH"' 2>/dev/null | tail -n 1 || true)"
append_path_dir "/bin"
append_path_dir "/usr/bin"
append_path_dir "/usr/local/bin"
[[ -z "$CLAUDE_PATH" ]] || append_path_dir "$(dirname "$CLAUDE_PATH")"
[[ -z "$CODEX_PATH" ]] || append_path_dir "$(dirname "$CODEX_PATH")"

ENV_TMP="$TMP_DIR/opencovibe.env"
{
  printf 'OPENCOVIBE_BIND="%s"\n' "$(env_escape "$BIND")"
  printf 'OPENCOVIBE_PORT="%s"\n' "$(env_escape "$PORT")"
  printf 'OPENCOVIBE_TOKEN="%s"\n' "$(env_escape "$TOKEN")"
  printf 'OPENCOVIBE_ALLOWED_ORIGINS="%s"\n' "$(env_escape "$ALLOWED_ORIGINS")"
  printf 'OPENCOVIBE_CLAUDE_PATH="%s"\n' "$(env_escape "$CLAUDE_PATH")"
  printf 'OPENCOVIBE_CODEX_PATH="%s"\n' "$(env_escape "$CODEX_PATH")"
  printf 'PATH="%s"\n' "$(env_escape "$SERVICE_PATH")"
  printf 'RUST_LOG="opencovibe_server=info,warn"\n'
} >"$ENV_TMP"
as_root install -m 0600 "$ENV_TMP" "$ENV_FILE"

SERVICE_TMP="$TMP_DIR/opencovibe.service"
cat >"$SERVICE_TMP" <<EOF
[Unit]
Description=OpenCovibe Web Server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$RUN_USER
WorkingDirectory="$(systemd_escape "$WORKSPACE")"
EnvironmentFile=$ENV_FILE
Environment="HOME=$(systemd_escape "$USER_HOME")"
ExecStart="$(systemd_escape "${PREFIX}/bin/opencovibe-server")"
Restart=on-failure
RestartSec=3
TimeoutStopSec=15
KillMode=mixed
NoNewPrivileges=true
PrivateTmp=true
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOF
as_root install -m 0644 "$SERVICE_TMP" "$SERVICE_FILE"
as_root systemctl daemon-reload

if [[ "$NO_START" == true ]]; then
  log "Installed without starting the service (--no-start)."
else
  as_root systemctl enable "$SERVICE_NAME"
  as_root systemctl restart "$SERVICE_NAME"
  if [[ "$BIND" == "::" || "$BIND" == "::1" ]]; then
    HEALTH_URL="http://[::1]:${PORT}/health"
  else
    HEALTH_URL="http://127.0.0.1:${PORT}/health"
  fi
  log "Waiting for $HEALTH_URL"
  HEALTHY=false
  for _ in {1..30}; do
    if curl --noproxy '*' -fsS --max-time 2 "$HEALTH_URL" >/dev/null 2>&1; then
      HEALTHY=true
      break
    fi
    sleep 1
  done
  if [[ "$HEALTHY" != true ]]; then
    as_root systemctl status "$SERVICE_NAME" --no-pager || true
    die "service did not become healthy; inspect logs with: journalctl -u $SERVICE_NAME -n 100"
  fi
fi

SERVER_IP="$(hostname -I 2>/dev/null | awk '{print $1}')"
[[ -n "$SERVER_IP" ]] || SERVER_IP="SERVER_IP"
log "Installed OpenCovibe $VERSION"
log "URL: http://${SERVER_IP}:${PORT}"
log "Token: $TOKEN"
log "Service user: $RUN_USER"
log "Workspace: $WORKSPACE"
log "No Claude Code or Codex files were installed or modified."
