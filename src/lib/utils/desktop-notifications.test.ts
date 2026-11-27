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
    expect(shouldDisplayNotice(decision({ permission: "unsupported" }))).toBe(false);
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
  it("falls back to the project basename when no run name is available", () => {
    const text = buildNotificationText(
      "complete",
      baseEvent("run_state", { state: "completed" }),
      { cwd: "/home/me/proj" },
      settings(),
    );
    expect(text.body).toContain("proj");
  });
});
