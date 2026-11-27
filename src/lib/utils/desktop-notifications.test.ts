import { afterEach, describe, it, expect, vi } from "vitest";
import type { BusEvent, NotificationSettings } from "$lib/types";
import {
  DEDUP_MS,
  buildNotificationText,
  classifyBusEvent,
  desktopStatusKind,
  eventAllowed,
  isActivelyViewing,
  nextDesktopToggleAction,
  notifyBusEvent,
  projectLabel,
  sendTest,
  setSettings,
  shouldDisplayNotice,
  summarizeContent,
  type DesktopNotificationState,
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
  it("uses the run name as the title and status plus reply as the body", () => {
    const text = buildNotificationText(
      "complete",
      baseEvent("run_state", { state: "completed" }),
      { runName: "修复登录", preview: "我修好了 `token` 刷新。\n\n并补了 3 个测试。" },
      settings(),
    );
    expect(text.title).toBe("修复登录");
    expect(text.body).toBe("已完成 · 我修好了 token 刷新。 并补了 3 个测试。");
  });
  it("builds English complete text", () => {
    const text = buildNotificationText(
      "complete",
      baseEvent("run_state", { state: "completed" }),
      { runName: "Fix login", preview: "Fixed the refresh flow." },
      settings({ language: "en" }),
    );
    expect(text.title).toBe("Fix login");
    expect(text.body).toBe("Completed · Fixed the refresh flow.");
  });
  it("falls back to a generic completion body without a preview", () => {
    const text = buildNotificationText(
      "complete",
      baseEvent("run_state", { state: "completed" }),
      { runName: "修复登录" },
      settings(),
    );
    expect(text.body).toBe("已完成 · 本轮对话已完成，等待你的下一步指令。");
  });
  it("uses the error message when present", () => {
    const text = buildNotificationText(
      "error",
      baseEvent("run_state", { state: "failed", error: "boom" }),
      undefined,
      settings(),
    );
    expect(text.title).toBe("OpenCovibe");
    expect(text.body).toBe("任务失败 · boom");
  });
  it("names the tool and its input for permission prompts", () => {
    const text = buildNotificationText(
      "permission",
      baseEvent("permission_prompt", {
        tool_name: "Bash",
        tool_input: { command: "npm run build" },
      }),
      { runName: "修复登录" },
      settings(),
    );
    expect(text.title).toBe("修复登录");
    expect(text.body).toBe("等待审批 · 工具调用 Bash：npm run build");
  });
  it("falls back to the project basename when no run name is available", () => {
    const text = buildNotificationText(
      "complete",
      baseEvent("run_state", { state: "completed" }),
      { cwd: "/home/me/proj", preview: "done" },
      settings(),
    );
    expect(text.title).toBe("proj");
    expect(text.body).toBe("已完成 · done");
  });
  it("truncates a long reply", () => {
    const text = buildNotificationText(
      "complete",
      baseEvent("run_state", { state: "completed" }),
      { runName: "长回复", preview: "字".repeat(200) },
      settings(),
    );
    expect(text.body.startsWith("已完成 · ")).toBe(true);
    expect(text.body.endsWith("…")).toBe(true);
    expect(text.body.length).toBeLessThan(140);
  });
});

describe("summarizeContent", () => {
  it("strips markdown and collapses whitespace", () => {
    expect(summarizeContent("## 标题\n\n**完成**了 `npm test`\n\n- 第一项")).toBe(
      "标题 完成了 npm test 第一项",
    );
  });
  it("drops fenced code blocks", () => {
    expect(summarizeContent("前面\n```js\nconst a = 1;\n```\n后面")).toBe("前面 后面");
  });
  it("truncates with an ellipsis", () => {
    expect(summarizeContent("a".repeat(200), 10)).toBe("aaaaaaaaaa…");
  });
  it("returns an empty string for empty input", () => {
    expect(summarizeContent("   \n  ")).toBe("");
  });
});

const state = (over: Partial<DesktopNotificationState> = {}): DesktopNotificationState => ({
  supported: true,
  secure: true,
  permission: "granted",
  enabled: false,
  ...over,
});

describe("desktopStatusKind", () => {
  it("reports insecure before unsupported because HTTP hides the API", () => {
    expect(desktopStatusKind(state({ supported: false, secure: false }))).toBe("insecure");
  });
  it("reports unsupported when secure but the API is missing", () => {
    expect(desktopStatusKind(state({ supported: false, secure: true }))).toBe("unsupported");
  });
  it("reports permission states", () => {
    expect(desktopStatusKind(state({ permission: "granted" }))).toBe("granted");
    expect(desktopStatusKind(state({ permission: "denied" }))).toBe("denied");
    expect(desktopStatusKind(state({ permission: "default" }))).toBe("default");
  });
});

describe("nextDesktopToggleAction", () => {
  it("disables when currently enabled", () => {
    expect(nextDesktopToggleAction(state({ enabled: true }))).toBe("disable");
  });
  it("does nothing when unusable", () => {
    expect(nextDesktopToggleAction(state({ supported: false }))).toBe("noop");
    expect(nextDesktopToggleAction(state({ secure: false }))).toBe("noop");
  });
  it("requests permission only from the default state", () => {
    expect(nextDesktopToggleAction(state({ permission: "default" }))).toBe("request");
    expect(nextDesktopToggleAction(state({ permission: "denied" }))).toBe("noop");
  });
  it("enables without re-requesting when permission is already granted", () => {
    expect(nextDesktopToggleAction(state({ permission: "granted" }))).toBe("enable");
  });
});

describe("isActivelyViewing", () => {
  it("only treats a visible and focused page as actively viewed", () => {
    expect(isActivelyViewing("visible", true)).toBe(true);
    expect(isActivelyViewing("visible", false)).toBe(false);
    expect(isActivelyViewing("hidden", true)).toBe(false);
    expect(isActivelyViewing("hidden", false)).toBe(false);
  });
});

describe("sendTest", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  function stubWindow(
    permission: string,
    opts: { secure?: boolean; missing?: boolean; throws?: boolean } = {},
  ) {
    const calls: unknown[][] = [];
    class FakeNotification {
      static permission = permission;
      static requestPermission = vi.fn(async () => permission);
      constructor(...args: unknown[]) {
        if (opts.throws) throw new Error("nope");
        calls.push(args);
      }
    }
    const win: Record<string, unknown> = {
      isSecureContext: opts.secure ?? true,
      localStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {} },
      location: { assign: vi.fn() },
      focus: vi.fn(),
    };
    if (!opts.missing) win.Notification = FakeNotification;
    vi.stubGlobal("window", win);
    return { calls };
  }

  it("returns unsupported when the API is missing", () => {
    stubWindow("granted", { missing: true });
    expect(sendTest()).toBe("unsupported");
  });
  it("returns insecure on a non-secure origin", () => {
    stubWindow("granted", { secure: false });
    expect(sendTest()).toBe("insecure");
  });
  it("returns denied without constructing a notification", () => {
    const { calls } = stubWindow("denied");
    expect(sendTest()).toBe("denied");
    expect(calls.length).toBe(0);
  });
  it("constructs a notification and reports sent", () => {
    const { calls } = stubWindow("granted");
    expect(sendTest()).toBe("sent");
    expect(calls.length).toBe(1);
  });
  it("reports unsupported when construction throws", () => {
    stubWindow("granted", { throws: true });
    expect(sendTest()).toBe("unsupported");
  });
});

describe("notification options", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  function stubNotifyWindow() {
    const calls: unknown[][] = [];
    class FakeNotification {
      static permission = "granted";
      static requestPermission = vi.fn(async () => "granted");
      onclick: (() => void) | null = null;
      constructor(...args: unknown[]) {
        calls.push(args);
      }
    }
    const win = {
      isSecureContext: true,
      Notification: FakeNotification,
      localStorage: {
        getItem: (key: string) => (key === "ocv:desktop-notifications" ? "1" : null),
        setItem: () => {},
        removeItem: () => {},
      },
      location: { assign: vi.fn() },
      focus: vi.fn(),
    };
    vi.stubGlobal("window", win);
    return { calls };
  }

  it("sendTest does not set a tag, so repeated tests always alert", () => {
    const { calls } = stubWindowForOptions("granted");
    expect(sendTest()).toBe("sent");
    const opts = calls[0][1] as Record<string, unknown>;
    expect(opts.tag).toBeUndefined();
  });

  it("run notifications do not set a tag, so every turn alerts", () => {
    const { calls } = stubNotifyWindow();
    setSettings(settings());
    notifyBusEvent(
      { type: "run_state", run_id: "run-tag-test", state: "completed" } as unknown as BusEvent,
      { runName: "Demo" },
    );
    expect(calls.length).toBe(1);
    const opts = calls[0][1] as Record<string, unknown>;
    expect(opts.tag).toBeUndefined();
  });
});

function stubWindowForOptions(permission: string) {
  const calls: unknown[][] = [];
  class FakeNotification {
    static permission = permission;
    static requestPermission = vi.fn(async () => permission);
    constructor(...args: unknown[]) {
      calls.push(args);
    }
  }
  vi.stubGlobal("window", {
    isSecureContext: true,
    Notification: FakeNotification,
    localStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {} },
    location: { assign: vi.fn() },
    focus: vi.fn(),
  });
  return { calls };
}
