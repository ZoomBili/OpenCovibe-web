/**
 * Browser desktop notifications.
 *
 * The Web UI runs on PC/mobile browsers, so the CLI's own terminal notification
 * channel (`preferredNotifChannel`) can never reach the user. This module turns
 * live bus events into system notifications using the browser Notification API.
 *
 * Design notes:
 * - The enable switch is device-local (`localStorage`): notification permission
 *   is granted per browser/device, so syncing it through user settings would be
 *   meaningless on other devices.
 * - Event filtering reuses the server-side notification settings (master switch
 *   plus on_complete / on_error / on_permission).
 * - Notifications only fire while the page is open and the run is subscribed;
 *   background runs and closed tabs are out of scope for the WS transport.
 */
import type { BusEvent, NotificationSettings } from "$lib/types";
import { dbg, dbgWarn } from "$lib/utils/debug";

export type DesktopNotifyKind = "complete" | "error" | "permission";
export type DesktopPermission = "default" | "granted" | "denied" | "unsupported";

export interface DesktopNotificationState {
  supported: boolean;
  secure: boolean;
  permission: DesktopPermission;
  enabled: boolean;
}

export interface DesktopNotifyContext {
  runName?: string;
  cwd?: string;
}

export interface DisplayDecision {
  supported: boolean;
  secure: boolean;
  permission: DesktopPermission;
  enabled: boolean;
  settings: NotificationSettings | null;
  kind: DesktopNotifyKind;
  visible: boolean;
  lastNotifiedAt?: number;
  now: number;
}

const STORAGE_KEY = "ocv:desktop-notifications";
export const DEDUP_MS = 10_000;

let cachedSettings: NotificationSettings | null = null;
const lastNotified = new Map<string, number>();

/** Map a live bus event to a notifiable category, or null when it is noise. */
export function classifyBusEvent(ev: BusEvent): DesktopNotifyKind | null {
  if (ev.type === "permission_prompt") return "permission";
  if (ev.type === "run_state") {
    if (ev.state === "idle" || ev.state === "completed") return "complete";
    if (ev.state === "failed") return "error";
  }
  return null;
}

/** Whether the user's server-side notification settings allow this category. */
export function eventAllowed(
  settings: NotificationSettings | null,
  kind: DesktopNotifyKind,
): boolean {
  if (!settings?.enabled) return false;
  switch (kind) {
    case "complete":
      return settings.on_complete;
    case "error":
      return settings.on_error;
    case "permission":
      return settings.on_permission;
  }
}

/**
 * Pure gate in front of `new Notification`.
 * `force` is used by the settings-page test button: it bypasses the visibility
 * and dedup checks so the user gets immediate feedback while looking at the page.
 */
export function shouldDisplayNotice(d: DisplayDecision, force = false): boolean {
  if (!d.supported || !d.secure) return false;
  if (!d.enabled || d.permission !== "granted") return false;
  if (!eventAllowed(d.settings, d.kind)) return false;
  if (!force) {
    if (d.visible) return false;
    if (d.lastNotifiedAt !== undefined && d.now - d.lastNotifiedAt < DEDUP_MS) return false;
  }
  return true;
}

/** Render the project directory the same way the server-side notifier does. */
export function projectLabel(cwd: string | undefined, mode: string): string {
  const value = (cwd ?? "").trim();
  if (!value) return "";
  if (mode === "full") return value;
  const parts = value.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? "";
}

export function buildNotificationText(
  kind: DesktopNotifyKind,
  ev: BusEvent,
  ctx: DesktopNotifyContext | undefined,
  settings: NotificationSettings | null,
): { title: string; body: string } {
  const zh = settings?.language !== "en";
  const project = projectLabel(ctx?.cwd, settings?.project_path_mode ?? "basename");
  const subject = (ctx?.runName ?? "").trim() || project;

  if (kind === "error") {
    const detail = ev.type === "run_state" && typeof ev.error === "string" ? ev.error.trim() : "";
    const fallback = zh
      ? subject
        ? `「${subject}」运行失败。`
        : "运行失败，请打开会话查看详情。"
      : subject
        ? `"${subject}" failed.`
        : "The run failed. Open the session for details.";
    return {
      title: zh ? "OpenCovibe · 任务失败" : "OpenCovibe · Task failed",
      body: detail || fallback,
    };
  }

  if (kind === "permission") {
    const tool = ev.type === "permission_prompt" ? ev.tool_name.trim() : "";
    let body: string;
    if (zh) {
      if (tool && subject) body = `「${subject}」的工具调用「${tool}」等待审批。`;
      else if (tool) body = `工具调用「${tool}」等待审批。`;
      else if (subject) body = `「${subject}」正在等待你批准工具调用。`;
      else body = "Agent 正在等待你批准工具调用。";
    } else {
      if (tool && subject)
        body = `"${subject}" is waiting for approval of the "${tool}" tool call.`;
      else if (tool) body = `The "${tool}" tool call is waiting for approval.`;
      else if (subject) body = `"${subject}" is waiting for tool approval.`;
      else body = "The agent is waiting for tool approval.";
    }
    return {
      title: zh ? "OpenCovibe · 等待审批" : "OpenCovibe · Approval required",
      body,
    };
  }

  const body = zh
    ? subject
      ? `「${subject}」本轮对话已完成。`
      : "本轮对话已完成，等待你的下一步指令。"
    : subject
      ? `"${subject}" finished this turn.`
      : "The agent finished this turn.";
  return {
    title: zh ? "OpenCovibe · 任务完成" : "OpenCovibe · Task completed",
    body,
  };
}

/** The Notification constructor, or null when unavailable/insecure. */
function notificationCtor(): typeof Notification | null {
  if (typeof window === "undefined") return null;
  if (!("Notification" in window)) return null;
  if (window.isSecureContext === false) return null;
  return window.Notification;
}

export function getState(): DesktopNotificationState {
  const supported = typeof window !== "undefined" && "Notification" in window;
  const secure = typeof window !== "undefined" && window.isSecureContext !== false;
  let permission: DesktopPermission = "unsupported";
  if (supported) {
    permission = (window.Notification.permission as DesktopPermission) ?? "default";
  }
  let enabled = false;
  if (typeof window !== "undefined") {
    try {
      enabled = window.localStorage.getItem(STORAGE_KEY) === "1";
    } catch {
      enabled = false;
    }
  }
  return { supported, secure, permission, enabled };
}

export function setEnabled(enabled: boolean): void {
  if (typeof window === "undefined") return;
  try {
    if (enabled) window.localStorage.setItem(STORAGE_KEY, "1");
    else window.localStorage.removeItem(STORAGE_KEY);
  } catch (e) {
    dbgWarn("desktop-notif", "persist enabled failed", e);
  }
}

/** Inject the server-side notification settings (switches, language, path mode). */
export function setSettings(settings: NotificationSettings | null): void {
  cachedSettings = settings;
}

export async function requestPermission(): Promise<DesktopPermission> {
  const Ctor = notificationCtor();
  if (!Ctor) return "unsupported";
  try {
    if (Ctor.permission === "granted") return "granted";
    return (await Ctor.requestPermission()) as DesktopPermission;
  } catch (e) {
    dbgWarn("desktop-notif", "requestPermission failed", e);
    return "denied";
  }
}

function display(title: string, body: string, runId: string, key: string, now: number): void {
  const Ctor = notificationCtor();
  if (!Ctor) return;
  try {
    const notice = new Ctor(title, { body, tag: `ocv-${key}`, data: { runId } });
    notice.onclick = () => {
      try {
        window.focus();
        window.location.assign(`/chat?run=${encodeURIComponent(runId)}`);
      } catch (e) {
        dbgWarn("desktop-notif", "navigate failed", e);
      }
    };
    lastNotified.set(key, now);
    dbg("desktop-notif", "shown", { key });
  } catch (e) {
    dbgWarn("desktop-notif", "display failed", e);
  }
}

export function notifyBusEvent(ev: BusEvent, ctx?: DesktopNotifyContext): void {
  const kind = classifyBusEvent(ev);
  if (!kind) return;

  const state = getState();
  const key = `${ev.run_id}:${kind}`;
  const now = Date.now();
  const visible = typeof document !== "undefined" && document.visibilityState === "visible";

  if (
    !shouldDisplayNotice({
      supported: state.supported,
      secure: state.secure,
      permission: state.permission,
      enabled: state.enabled,
      settings: cachedSettings,
      kind,
      visible,
      lastNotifiedAt: lastNotified.get(key),
      now,
    })
  ) {
    return;
  }

  const { title, body } = buildNotificationText(kind, ev, ctx, cachedSettings);
  display(title, body, ev.run_id, key, now);
}

/** Settings-page test button: ignores server switches, visibility and dedup. */
export function sendTest(): void {
  const Ctor = notificationCtor();
  if (!Ctor || Ctor.permission !== "granted") return;
  const zh = cachedSettings?.language !== "en";
  try {
    new Ctor(zh ? "OpenCovibe · 测试通知" : "OpenCovibe · Test notification", {
      body: zh
        ? "如果你看到这条通知，浏览器桌面通知已生效。"
        : "If you see this, browser desktop notifications work.",
      tag: "ocv-test",
    });
  } catch (e) {
    dbgWarn("desktop-notif", "test display failed", e);
  }
}
