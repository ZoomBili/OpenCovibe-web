# 浏览器桌面通知 + 隐藏 CLI 终端通知设置

日期：2026-11-27
状态：待批准

## 背景与问题

设置 → CLI 配置 → 高级里有一项「通知方式」（`preferredNotifChannel`，文案「桌面通知的推送方式」），
选项为 `auto` / `iterm2` / `terminal_bell`。它通过 `updateCliConfig` 原样写进
`~/.claude/settings.json`，由 Claude Code CLI 在**它所在的终端**里执行（iTerm2 转义序列、终端响铃）。

OpenCovibe-web 的架构是：`claude` 作为 Rust 服务端子进程运行，没有连接到用户的终端，
浏览器/手机与这个终端之间没有任何通道。前端 `src/` 里也没有任何 `Notification` API 的使用。
因此该设置在 Web 形态下是死配置——不是坏了，是从设计上不会生效。

## 目标

1. 从 CLI 配置页移除这个无意义的设置，避免误导。
2. 在「通知」页签提供真正能在 PC 浏览器和手机浏览器上工作的桌面通知，
   覆盖与现有企业微信/HTTP 通道相同的三类事件：完成、失败、等待审批。

## 非目标

- 不实现 Web Push / Service Worker。浏览器通知只在页面仍打开、且 WS 订阅仍然存活时触发。
- 不为后台（未订阅）的 run 推送通知。当前 WS 只向订阅了该 run 的客户端发送 bus-event；
  要覆盖后台 run 需要后端推送设施，超出本次范围。
- 不改动服务端通知（企业微信 / HTTP）的行为与协议。
- 不支持非安全上下文（`http://IP`）。`Notification` API 在非安全上下文不存在，
  卡片需明确提示并要求 HTTPS / localhost，不支持时禁用开关。

## 约束

- Notification 权限按浏览器/设备授予，因此开关与权限状态是**设备本地**的（`localStorage`），
  不写入服务端 `NotificationSettings`。
- 事件筛选复用服务端已有的总开关与 `on_complete` / `on_error` / `on_permission`，不新增一套事件勾选。
- 授权必须由用户手势触发（点击开关时调用 `requestPermission`）。
- iOS Safari 仅「添加到主屏幕」的 PWA 支持通知；Android Chrome 正常。UI 需给出提示。
- 页面可见时不弹系统通知，避免用户正在看页面时被打扰。

## 方案（A，已选定）

对比过两种做法：

- **A. 设备本地（选定）**：开关存 `localStorage`，权限状态读浏览器。无需改 Rust。
- B. 服务端同步：给 `NotificationSettings` 加 `desktop` 字段并同步 serde 契约。
  权限本身按设备授予，同步过去无法兑现，还要改后端与契约测试，故不采用。

### 1. 隐藏 CLI 设置

- 删除 `src/routes/settings/+page.svelte` 中 `CLI_CONFIG_SETTINGS` 的 `preferredNotifChannel` 条目。
- 删除 `messages/zh-CN.json` / `messages/en.json` 中仅被它使用的
  `settings_cliConfig_notifChannelLabel`、`settings_cliConfig_notifChannelDesc`、
  `settings_cliConfig_optIterm2`、`settings_cliConfig_optTerminalBell`。
  （`settings_cliConfig_optAuto` 仍被 `teammateMode` 使用，保留。）

### 2. 新增 `src/lib/utils/desktop-notifications.ts`

状态与 API：

- `setSettings(notifications: NotificationSettings)`：缓存服务端的总开关、三类事件开关、
  `language`、`project_path_mode`。
- `setEnabled(enabled: boolean)` / `isEnabled()`：设备本地开关，
  存储键 `ocv:desktop-notifications`（值 `"1"` / 不存在）。
- `getState()`：返回 `{ supported, secure, permission, enabled }`，供 UI 渲染。
- `requestPermission(): Promise<NotificationPermission | "unsupported">`。
- `notifyBusEvent(ev: BusEvent, ctx?: { runName?: string; cwd?: string })`。
- `sendTest()`：即使页面可见也弹一条测试通知。

事件映射（与服务端 `notifications.rs` 对齐）：

| BusEvent | 类别 | 使用的事件开关 |
| --- | --- | --- |
| `run_state` + `idle` / `completed` | 完成 | `on_complete` |
| `run_state` + `failed` | 失败 | `on_error` |
| `permission_prompt` | 等待审批 | `on_permission` |

触发条件（全部满足）：

1. `getState().supported && secure`；
2. 设备开关已开；
3. `Notification.permission === "granted"`；
4. 服务端总开关 `enabled` 与该类别事件开关为真；
5. `document.visibilityState !== "visible"`（测试通知除外）；
6. 同一 `run_id` + 类别在 10 秒内未弹过（去重，防止 `idle`/`permission_prompt` 重复）。

文案：按 `language`（`"en"` → 英文，其余 → 中文）生成，与现有通知设置一致；
标题 `OpenCovibe · 任务完成 / 任务失败 / 等待审批`，正文带 run 名或项目目录与工具名。
不使用 `t()`，避免与「通知语言」设置冲突。

点击通知：`window.focus()`，然后 `location.assign("/chat?run=" + encodeURIComponent(runId))`。

可测试性：把「是否该弹」「生成什么标题/正文」拆成纯函数导出，副作用（`new Notification`、
`localStorage`、`document`）集中在薄封装里，node 测试环境可直接验证纯函数。

### 3. 接入点

- `src/lib/stores/event-middleware.ts`：`_handleBusEvent` 中调用
  `notifyBusEvent(ev, store ? { runName: store.run?.name, cwd: store.run?.cwd } : undefined)`。
  放在 store 查找之前或之后均可，通知器不依赖 store。
- `src/routes/+layout.svelte`：`loadSettings()` 拿到设置后调用
  `desktopNotifications.setSettings(settings.notifications)`。
- `src/lib/components/NotificationSettingsPanel.svelte`：顶部新增「浏览器桌面通知」卡片。
  - 开关：点击时若未授权则先 `requestPermission()`，成功后再开启并持久化；
  - 状态行：已授权 / 未授权 / 已被拒绝 / 不支持 / 需要 HTTPS；
  - 「发送测试通知」按钮；
  - 提示：仅本浏览器生效；页面关闭后不再通知；iOS 需添加到主屏幕。

### 4. 测试

- `src/lib/utils/desktop-notifications.test.ts`（node 环境）：
  - 事件 → 类别的映射；
  - 总开关 / 单事件开关关闭时不弹；
  - `visibilityState === "visible"` 时不弹；
  - 10 秒内重复事件被去重；
  - 文案按 `language` 生成。
- `src/lib/stores/event-middleware.test.ts`：mock 通知模块，断言 bus 事件会调用
  `notifyBusEvent`。

## 风险

- 用户停留在非聊天页时，聊天页已卸载并取消订阅，此时不会收到 bus-event，也就不会通知。
  这是当前 WS 订阅模型的限制，需在卡片说明「页面需保持打开」。
- 手机浏览器在锁屏/切后台后可能被系统挂起 JS，通知可能延迟或不弹。无法通过纯前端解决。
