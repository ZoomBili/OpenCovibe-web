# 浏览器桌面通知 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 移除 Web 版里无效的 CLI 终端通知设置，并让浏览器在会话完成、失败、等待审批时弹出真正的系统通知。

**Architecture:** 新增一个纯前端模块 `desktop-notifications.ts`，复用服务端的通知总开关与三类事件开关，设备本地的启用开关与权限状态存 `localStorage`/浏览器；`EventMiddleware` 在收到 bus-event 时调用它，`+layout.svelte` 注入设置，`NotificationSettingsPanel` 提供开关与测试按钮。

**Tech Stack:** SvelteKit 2 / Svelte 5 runes / TypeScript / Vitest（node 环境）/ 浏览器 Notification API。

**Spec:** `docs/superpowers/specs/2026-11-27-browser-desktop-notifications-design.md`

## Global Constraints

- Notification API 只在安全上下文可用（HTTPS / localhost）；不支持时禁用开关并提示。
- 设备本地存储键固定为 `ocv:desktop-notifications`，值为 `"1"` 表示开启。
- 同 run 同类别 10 秒内去重（`DEDUP_MS = 10_000`）。
- 页面可见（`document.visibilityState === "visible"`）时不弹，测试通知除外。
- 不新增服务端字段、不改 Rust、不改 `NotificationSettings` 契约。
- 文案按 `settings.language === "en"` 分支，不使用 `t()`。
- 所有新增 UI 文案必须同时写入 `messages/en.json` 与 `messages/zh-CN.json`。

## Review Focus

- 浏览器不支持 / 非 HTTPS 时点击开关：应保持关闭并显示明确原因，不能抛异常。
- 权限被用户拒绝后再次点击开关：应提示去站点设置，而不是反复调用 `requestPermission`。
- 同一个 `run_state: idle` 重复到达：10 秒内只弹一次。
- 用户正在看页面（前台标签页）：不弹系统通知，避免干扰。
- 通配/旧版设置缺少 `notifications` 字段：通知器应按关闭处理，不崩溃。

---

### Task 1: 通知核心模块与纯函数测试

**Files:**
- Create: `src/lib/utils/desktop-notifications.ts`
- Test: `src/lib/utils/desktop-notifications.test.ts`

**Interfaces:**
- Consumes: `BusEvent`, `NotificationSettings` from `$lib/types`。
- Produces:
  - `type DesktopNotifyKind = "complete" | "error" | "permission"`
  - `type DesktopPermission = "default" | "granted" | "denied" | "unsupported"`
  - `interface DesktopNotificationState { supported: boolean; secure: boolean; permission: DesktopPermission; enabled: boolean }`
  - `interface DesktopNotifyContext { runName?: string; cwd?: string }`
  - `interface DisplayDecision { supported: boolean; secure: boolean; permission: DesktopPermission; enabled: boolean; settings: NotificationSettings | null; kind: DesktopNotifyKind; visible: boolean; lastNotifiedAt?: number; now: number }`
  - `const DEDUP_MS = 10_000`
  - `function classifyBusEvent(ev: BusEvent): DesktopNotifyKind | null`
  - `function eventAllowed(settings: NotificationSettings | null, kind: DesktopNotifyKind): boolean`
  - `function shouldDisplayNotice(d: DisplayDecision, force?: boolean): boolean`
  - `function projectLabel(cwd: string | undefined, mode: string): string`
  - `function buildNotificationText(kind: DesktopNotifyKind, ev: BusEvent, ctx: DesktopNotifyContext | undefined, settings: NotificationSettings | null): { title: string; body: string }`
  - `function getState(): DesktopNotificationState`
  - `function setEnabled(enabled: boolean): void`
  - `function setSettings(settings: NotificationSettings | null): void`
  - `function requestPermission(): Promise<DesktopPermission>`
  - `function notifyBusEvent(ev: BusEvent, ctx?: DesktopNotifyContext): void`
  - `function sendTest(): void`

- [ ] **Step 1: 写失败测试**

创建 `src/lib/utils/desktop-notifications.test.ts`，覆盖：

```ts
import { describe, it, expect } from "vitest";
import type { BusEvent, NotificationSettings } from "$lib/types";
import {
  DEDUP_MS,
  buildNotificationText,
  classifyBusEvent,
  eventAllowed,
  projectLabel,
  shouldDisplayNotice,
  type DisplayDecision,
} from "./desktop-notifications";

const baseEvent = (type: string, extra: Record<string, unknown> = {}) =>
  ({ type, run_id: "run-1", ...extra }) as unknown as BusEvent;

const settings = (over: Partial<NotificationSettings> = {}): NotificationSettings => ({
  enabled: true,
  on_complete: true,
  on_error: true,
  on_permission: true,
  language: "zh-CN",
  project_path_mode: "basename",
  wecom: { enabled: false, webhook_url: "" },
  http: { enabled: false, url: "", method: "POST" },
  ...over,
});

const decision = (over: Partial<DisplayDecision> = {}): DisplayDecision => ({
  supported: true,
  secure: true,
  permission: "granted",
  enabled: true,
  settings: settings(),
  kind: "complete",
  visible: false,
  now: 100_000,
  ...over,
});

describe("classifyBusEvent", () => {
  it("maps idle/completed to complete", () => {
    expect(classifyBusEvent(baseEvent("run_state", { state: "idle" }))).toBe("complete");
    expect(classifyBusEvent(baseEvent("run_state", { state: "completed" }))).toBe("complete");
  });
  it("maps failed to error", () => {
    expect(classifyBusEvent(baseEvent("run_state", { state: "failed" }))).toBe("error");
  });
  it("maps permission_prompt to permission", () => {
    expect(classifyBusEvent(baseEvent("permission_prompt", { tool_name: "Bash" }))).toBe(
      "permission",
    );
  });
  it("ignores other events", () => {
    expect(classifyBusEvent(baseEvent("run_state", { state: "running" }))).toBeNull();
    expect(classifyBusEvent(baseEvent("message_delta"))).toBeNull();
  });
});

describe("eventAllowed", () => {
  it("requires master switch", () => {
    expect(eventAllowed(settings({ enabled: false }), "complete")).toBe(false);
  });
  it("respects per-event switches", () => {
    expect(eventAllowed(settings({ on_complete: false }), "complete")).toBe(false);
    expect(eventAllowed(settings({ on_error: false }), "error")).toBe(false);
    expect(eventAllowed(settings({ on_permission: false }), "permission")).toBe(false);
  });
  it("treats missing settings as off", () => {
    expect(eventAllowed(null, "complete")).toBe(false);
  });
});

describe("shouldDisplayNotice", () => {
  it("requires support, secure context, enabled and granted", () => {
    expect(shouldDisplayNotice(decision({ supported: false }))).toBe(false);
    expect(shouldDisplayNotice(decision({ secure: false }))).toBe(false);
    expect(shouldDisplayNotice(decision({ enabled: false }))).toBe(false);
    expect(shouldDisplayNotice(decision({ permission: "default" }))).toBe(false);
    expect(shouldDisplayNotice(decision({ permission: "denied" }))).toBe(false);
  });
  it("suppresses while the page is visible", () => {
    expect(shouldDisplayNotice(decision({ visible: true }))).toBe(false);
    expect(shouldDisplayNotice(decision({ visible: true }), true)).toBe(true);
  });
  it("dedupes within 10 seconds", () => {
    expect(shouldDisplayNotice(decision({ lastNotifiedAt: 95_000 }))).toBe(false);
    expect(shouldDisplayNotice(decision({ lastNotifiedAt: 90_000 }))).toBe(true);
    expect(DEDUP_MS).toBe(10_000);
  });
});

describe("projectLabel", () => {
  it("returns basename or full path", () => {
    expect(projectLabel("/home/me/proj", "basename")).toBe("proj");
    expect(projectLabel("/home/me/proj/", "basename")).toBe("proj");
    expect(projectLabel("/home/me/proj", "full")).toBe("/home/me/proj");
  });
  it("handles empty cwd", () => {
    expect(projectLabel(undefined, "basename")).toBe("");
    expect(projectLabel("", "full")).toBe("");
  });
});

describe("buildNotificationText", () => {
  it("builds Chinese complete text with run name", () => {
    const text = buildNotificationText(
      "complete",
      baseEvent("run_state", { state: "completed" }),
      { runName: "修复登录" },
      settings(),
    );
    expect(text.title).toBe("OpenCovibe · 任务完成");
    expect(text.body).toContain("修复登录");
  });
  it("builds English complete text", () => {
    const text = buildNotificationText(
      "complete",
      baseEvent("run_state", { state: "completed" }),
      { runName: "Fix login" },
      settings({ language: "en" }),
    );
    expect(text.title).toBe("OpenCovibe · Task completed");
    expect(text.body).toContain("Fix login");
  });
  it("uses the error message when present", () => {
    const text = buildNotificationText(
      "error",
      baseEvent("run_state", { state: "failed", error: "boom" }),
      undefined,
      settings(),
    );
    expect(text.title).toBe("OpenCovibe · 任务失败");
    expect(text.body).toBe("boom");
  });
  it("names the tool for permission prompts", () => {
    const text = buildNotificationText(
      "permission",
      baseEvent("permission_prompt", { tool_name: "Bash" }),
      { runName: "修复登录" },
      settings(),
    );
    expect(text.title).toBe("OpenCovibe · 等待审批");
    expect(text.body).toContain("Bash");
  });
});
```

- [ ] **Step 2: 运行测试确认失败**

Run: `npx vitest run src/lib/utils/desktop-notifications.test.ts`
Expected: FAIL，模块或导出不存在。

- [ ] **Step 3: 实现 `src/lib/utils/desktop-notifications.ts`**

实现纯函数；副作用封装在导出函数内：

- `getState()`：`typeof window === "undefined"` 时 `supported=false`；否则 `"Notification" in window` 且 `window.isSecureContext !== false`；`permission` 读 `Notification.permission`；`enabled` 读 `localStorage.getItem("ocv:desktop-notifications") === "1"`。
- `setEnabled`：写入/移除 localStorage 键。
- `setSettings`：更新模块级 `cachedSettings`。
- `requestPermission`：不支持或不安全上下文返回 `"unsupported"`；否则 `await Notification.requestPermission()` 并返回。
- `notifyBusEvent`：`classifyBusEvent` → 取 `cachedSettings` → 读 `lastNotified[`${runId}:${kind}`]` → `shouldDisplayNotice` → `new Notification(title, { body, tag, data })`，设置 `onclick` 为聚焦并 `location.assign("/chat?run=" + encodeURIComponent(runId))`，然后记录时间。
- `sendTest`：不依赖服务端设置，仅要求支持/安全/已授权，弹一条固定测试通知；失败时 `dbgWarn`。
- 语言与文案按 spec；中文标题/正文格式见测试。

- [ ] **Step 4: 运行测试确认通过**

Run: `npx vitest run src/lib/utils/desktop-notifications.test.ts`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/lib/utils/desktop-notifications.ts src/lib/utils/desktop-notifications.test.ts
git commit -m "feat: add browser desktop notification core"
```

---

### Task 2: 接入 EventMiddleware

**Files:**
- Modify: `src/lib/stores/event-middleware.ts`（`_handleBusEvent`）
- Test: `src/lib/stores/event-middleware.test.ts`

**Interfaces:**
- Consumes: `notifyBusEvent(ev, ctx?)` from Task 1。
- Produces: 无新导出。

- [ ] **Step 1: 写失败测试**

在 `event-middleware.test.ts` 增加 mock：

```ts
vi.mock("$lib/utils/desktop-notifications", () => ({
  notifyBusEvent: vi.fn(),
}));
```

导入 `notifyBusEvent`，新增测试：`start()` 后 `fireBusEvent(makeBusEvent("run-1", "run_state", { state: "completed" }))`，断言 `notifyBusEvent` 被调用一次，且第一个参数 `type === "run_state"`。

- [ ] **Step 2: 运行测试确认失败**

Run: `npx vitest run src/lib/stores/event-middleware.test.ts`
Expected: FAIL，`notifyBusEvent` 调用次数为 0。

- [ ] **Step 3: 在 `_handleBusEvent` 中调用**

在 `this._trackAttention(ev);` 之后：

```ts
const store = this._subscriptions.get(ev.run_id);
notifyBusEvent(ev, store ? { runName: store.run?.name, cwd: store.run?.cwd } : undefined);
```

保留原有的 store 查找/批处理逻辑不变（可复用已取的 store 变量）。

- [ ] **Step 4: 运行测试确认通过**

Run: `npx vitest run src/lib/stores/event-middleware.test.ts`
Expected: PASS（含已有测试）。

- [ ] **Step 5: 提交**

```bash
git add src/lib/stores/event-middleware.ts src/lib/stores/event-middleware.test.ts
git commit -m "feat: dispatch browser notifications from event middleware"
```

---

### Task 3: 设置面板卡片、layout 注入与文案

**Files:**
- Modify: `src/lib/components/NotificationSettingsPanel.svelte`
- Modify: `src/routes/+layout.svelte`（`loadSettings`）
- Modify: `messages/zh-CN.json`
- Modify: `messages/en.json`

**Interfaces:**
- Consumes: Task 1 的 `getState` / `setEnabled` / `setSettings` / `requestPermission` / `sendTest` / `DesktopNotificationState`。
- Produces: 无新导出。

- [ ] **Step 1: 加 i18n 键（两个文件同步）**

| key | en | zh-CN |
| --- | --- | --- |
| `settings_notifications_desktop` | Browser desktop notifications | 浏览器桌面通知 |
| `settings_notifications_desktopDesc` | Show system notifications for completed, failed, and approval-waiting runs. Applies to this browser only. | 在系统通知中心提示完成、失败和等待审批的会话。仅对当前浏览器生效。 |
| `settings_notifications_desktopEnable` | Enable browser desktop notifications | 启用浏览器桌面通知 |
| `settings_notifications_desktopRequest` | Request permission | 请求授权 |
| `settings_notifications_desktopTest` | Send test notification | 发送测试通知 |
| `settings_notifications_desktopGranted` | Permission granted | 已授权 |
| `settings_notifications_desktopDefault` | Permission not requested | 未授权 |
| `settings_notifications_desktopDenied` | Blocked by the browser. Allow notifications in site settings. | 已被浏览器拒绝，请在站点设置中允许通知 |
| `settings_notifications_desktopUnsupported` | This browser does not support notifications | 当前浏览器不支持通知 |
| `settings_notifications_desktopInsecure` | Requires HTTPS or localhost | 需要 HTTPS 或 localhost 访问 |
| `settings_notifications_desktopHint` | The page must stay open; notifications stop after you close it or navigate to another app page. iOS requires adding to the Home Screen. | 页面需保持打开；关闭页面或切到应用内其他页面后不再通知。iOS 需先“添加到主屏幕”。 |

- [ ] **Step 2: 在 `NotificationSettingsPanel.svelte` 顶部加卡片**

- 导入 Task 1 的函数与类型。
- `let desktopState = $state(getState());`
- `onMount` 中 `refreshDesktopState()`；`applyToForm(next)` 后调用 `setSettings(next)`。
- 卡片放在总开关卡片之前，包含：
  - 标题/描述；
  - 开关按钮（`role="switch"`, `aria-checked={desktopState.enabled}`）；
  - 状态行按 `!supported → Unsupported`、`!secure → Insecure`、`permission` 映射；
  - 未授权且可用时显示「请求授权」按钮；
  - 已授权时显示「发送测试通知」。
- 开关点击逻辑：已开 → `setEnabled(false)`；未开 → 若可用且未授权先 `await requestPermission()`，仅当返回 `"granted"` 才 `setEnabled(true)`；每步后 `desktopState = getState()`。

- [ ] **Step 3: 在 `+layout.svelte` 的 `loadSettings` 注入**

`settings = await getUserSettings();` 之后调用 `desktopNotifications.setSettings(settings.notifications ?? null)`；模块用 `import * as desktopNotifications from "$lib/utils/desktop-notifications";` 引入。

- [ ] **Step 4: 验证**

Run: `npm run i18n:check && npx svelte-check --tsconfig ./tsconfig.json`
Expected: i18n 无 error；svelte-check 0 errors。

- [ ] **Step 5: 提交**

```bash
git add src/lib/components/NotificationSettingsPanel.svelte src/routes/+layout.svelte messages/en.json messages/zh-CN.json
git commit -m "feat: add browser notification settings UI"
```

---

### Task 4: 移除 CLI 终端通知设置

**Files:**
- Modify: `src/routes/settings/+page.svelte`（`CLI_CONFIG_SETTINGS`）
- Modify: `messages/zh-CN.json`
- Modify: `messages/en.json`

- [ ] **Step 1: 删除设置条目**

删除 `preferredNotifChannel` 的整个对象（`settings/+page.svelte:983-996`）。

- [ ] **Step 2: 删除专用 i18n key**

两个 locale 同步删除：`settings_cliConfig_notifChannelLabel`、`settings_cliConfig_notifChannelDesc`、`settings_cliConfig_optIterm2`、`settings_cliConfig_optTerminalBell`。保留 `settings_cliConfig_optAuto`（`teammateMode` 仍在用）。

- [ ] **Step 3: 验证**

Run: `grep -rn "preferredNotifChannel\|notifChannelLabel\|optIterm2\|optTerminalBell" src messages`
Expected: 无输出。
Run: `npm run i18n:check`
Expected: 通过。

- [ ] **Step 4: 提交**

```bash
git add src/routes/settings/+page.svelte messages/en.json messages/zh-CN.json
git commit -m "fix: drop CLI terminal notification setting from web UI"
```

---

### Task 5: 全量验证

- [ ] **Step 1: 运行项目检查**

```bash
npm run lint
npm run format:check
npm run check
npm run i18n:check
npm test
npm run build
```

Expected: 全部通过。

- [ ] **Step 2: 手动冒烟（HTTPS 环境）**

1. 打开设置 → 通知，开启「浏览器桌面通知」并授权；
2. 点「发送测试通知」，应出现系统通知；
3. 在聊天页发起一轮对话，切到别的应用，完成后应收到通知；
4. 设置 → CLI 配置 → 高级，确认「通知方式」已消失。

- [ ] **Step 3: 提交剩余格式修正（如有）**

```bash
git add -A
git commit -m "chore: format browser notification changes"
```
