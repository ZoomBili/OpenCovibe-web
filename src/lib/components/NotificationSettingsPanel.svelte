<script lang="ts">
  import { onMount } from "svelte";
  import * as api from "$lib/api";
  import type { NotificationSettings, NotificationTestResult } from "$lib/types";
  import { t } from "$lib/i18n/index.svelte";
  import { dbg, dbgWarn } from "$lib/utils/debug";
  import {
    desktopStatusKind,
    getState,
    nextDesktopToggleAction,
    requestPermission,
    sendTest,
    setEnabled,
    setSettings,
    type DesktopNotificationState,
    type DesktopTestResult,
  } from "$lib/utils/desktop-notifications";
  import Card from "$lib/components/Card.svelte";
  import Button from "$lib/components/Button.svelte";
  import Input from "$lib/components/Input.svelte";

  let { onSaved }: { onSaved?: (settings: NotificationSettings) => void } = $props();

  function defaults(): NotificationSettings {
    return {
      enabled: false,
      on_complete: true,
      on_error: true,
      on_permission: true,
      language: "zh-CN",
      project_path_mode: "basename",
      link_base_url: undefined,
      wecom: { enabled: false, webhook_url: "", mentioned_mobiles: [] },
      http: { enabled: false, url: "", method: "POST", headers: [], body_template: "" },
    };
  }

  /** Fill fields that older settings files / partial payloads may omit. */
  function normalize(raw?: NotificationSettings | null): NotificationSettings {
    const d = defaults();
    if (!raw) return d;
    return {
      ...d,
      ...raw,
      project_path_mode: raw.project_path_mode || "basename",
      wecom: { ...d.wecom, ...(raw.wecom ?? {}) },
      http: {
        ...d.http,
        ...(raw.http ?? {}),
        headers: (raw.http?.headers ?? []).map((h) => ({ key: h.key ?? "", value: h.value ?? "" })),
      },
      link_base_url: raw.link_base_url ?? undefined,
    };
  }

  let draft = $state<NotificationSettings>(defaults());
  let mobilesText = $state("");
  let linkBaseText = $state("");
  let loading = $state(true);
  let saving = $state(false);
  let saveMessage = $state("");
  let saveError = $state("");
  let testChannel = $state<"wecom" | "http" | null>(null);
  let testResult = $state<{ channel: "wecom" | "http"; result: NotificationTestResult } | null>(
    null,
  );
  let desktopState = $state<DesktopNotificationState>(getState());
  let desktopTestResult = $state<DesktopTestResult | null>(null);

  function refreshDesktopState() {
    desktopState = getState();
  }

  function canUseDesktop(): boolean {
    return desktopState.supported && desktopState.secure;
  }

  function desktopStatusLabel(): string {
    switch (desktopStatusKind(desktopState)) {
      case "unsupported":
        return t("settings_notifications_desktopUnsupported");
      case "insecure":
        return t("settings_notifications_desktopInsecure");
      case "granted":
        return t("settings_notifications_desktopGranted");
      case "denied":
        return t("settings_notifications_desktopDenied");
      default:
        return t("settings_notifications_desktopDefault");
    }
  }

  async function toggleDesktop() {
    desktopTestResult = null;
    const action = nextDesktopToggleAction(desktopState);
    if (action === "disable") {
      setEnabled(false);
      refreshDesktopState();
      return;
    }
    if (action === "noop") return;
    if (action === "request") {
      const result = await requestPermission();
      refreshDesktopState();
      if (result !== "granted") return;
    }
    setEnabled(true);
    refreshDesktopState();
  }

  async function requestDesktopPermission() {
    await requestPermission();
    refreshDesktopState();
  }

  function runDesktopTest() {
    desktopTestResult = sendTest();
  }

  function applyToForm(next: NotificationSettings) {
    draft = next;
    mobilesText = (next.wecom.mentioned_mobiles ?? []).join(", ");
    linkBaseText = next.link_base_url ?? "";
    setSettings(next);
  }

  onMount(async () => {
    refreshDesktopState();
    try {
      const settings = await api.getUserSettings();
      applyToForm(normalize(settings.notifications));
    } catch (e) {
      dbgWarn("notifications", "load failed", e);
      saveError = t("settings_notifications_loadFailed", { error: String(e) });
    } finally {
      loading = false;
    }
  });

  function parsedMobiles(): string[] {
    return mobilesText
      .split(/[,，\s]+/)
      .map((s) => s.trim())
      .filter(Boolean);
  }

  /** Build the payload we save/test from the current form. */
  function draftForRequest(): NotificationSettings {
    return {
      ...draft,
      link_base_url: linkBaseText.trim() || undefined,
      wecom: {
        ...draft.wecom,
        webhook_url: draft.wecom.webhook_url.trim(),
        mentioned_mobiles: parsedMobiles(),
      },
      http: {
        ...draft.http,
        url: draft.http.url.trim(),
        method: draft.http.method || "POST",
        headers: (draft.http.headers ?? []).filter((h) => h.key.trim() !== ""),
        body_template: draft.http.body_template ?? "",
      },
    };
  }

  async function save() {
    saveMessage = "";
    saveError = "";

    const isHttpUrl = (value: string) => {
      try {
        const u = new URL(value);
        return u.protocol === "http:" || u.protocol === "https:";
      } catch {
        return false;
      }
    };
    const pre = draftForRequest();
    if (pre.wecom.enabled && pre.wecom.webhook_url && !isHttpUrl(pre.wecom.webhook_url)) {
      saveError = t("settings_notifications_invalidUrl");
      return;
    }
    if (pre.http.enabled && pre.http.url && !isHttpUrl(pre.http.url)) {
      saveError = t("settings_notifications_invalidUrl");
      return;
    }

    saving = true;
    try {
      const saved = await api.updateUserSettings({ notifications: pre });
      applyToForm(normalize(saved.notifications));
      onSaved?.(normalize(saved.notifications));
      saveMessage = t("settings_notifications_saved");
      dbg("notifications", "saved");
    } catch (e) {
      dbgWarn("notifications", "save failed", e);
      saveError = t("settings_notifications_saveFailed", { error: String(e) });
    } finally {
      saving = false;
    }
  }

  async function runTest(channel: "wecom" | "http") {
    if (testChannel) return;
    testChannel = channel;
    testResult = null;
    try {
      const result = await api.testNotificationChannel(channel, draftForRequest());
      testResult = { channel, result };
    } catch (e) {
      dbgWarn("notifications", "test failed", e);
      testResult = { channel, result: { ok: false, message: String(e) } };
    } finally {
      testChannel = null;
    }
  }

  function addHeader() {
    draft.http.headers = [...(draft.http.headers ?? []), { key: "", value: "" }];
  }

  function removeHeader(index: number) {
    draft.http.headers = (draft.http.headers ?? []).filter((_, i) => i !== index);
  }

  const PLACEHOLDERS = [
    "{event}",
    "{title}",
    "{status}",
    "{status_label}",
    "{run_id}",
    "{run_name}",
    "{session}",
    "{turn}",
    "{turn_preview}",
    "{project}",
    "{cwd}",
    "{agent}",
    "{message}",
    "{duration}",
    "{tokens_m}",
    "{run_url}",
    "{time}",
    "{text}",
    "{text_body}",
  ];

  /** A valid JSON template. Placeholders MUST be inside quotes. */
  const HTTP_BODY_EXAMPLE = `{
  "title": "🤖 AI Coding",
  "content": "{text_body}",
  "type": "text"
}`;

  function fillBodyExample() {
    draft.http.body_template = HTTP_BODY_EXAMPLE;
  }

  const EVENT_OPTIONS = [
    {
      key: "on_complete",
      label: "settings_notifications_onComplete",
      desc: "settings_notifications_onCompleteDesc",
    },
    {
      key: "on_error",
      label: "settings_notifications_onError",
      desc: "settings_notifications_onErrorDesc",
    },
    {
      key: "on_permission",
      label: "settings_notifications_onPermission",
      desc: "settings_notifications_onPermissionDesc",
    },
  ] as const;
</script>

<div class="space-y-6">
  {#if loading}
    <Card class="p-6">
      <p class="text-sm text-muted-foreground">{t("settings_notifications_loading")}</p>
    </Card>
  {:else}
    <!-- Browser desktop notifications (device-local) -->
    <Card class="p-4 sm:p-6 space-y-4">
      <div class="flex items-start justify-between gap-4">
        <div>
          <h2 class="text-sm font-semibold text-muted-foreground uppercase tracking-wider">
            {t("settings_notifications_desktop")}
          </h2>
          <p class="mt-1 text-xs text-muted-foreground">
            {t("settings_notifications_desktopDesc")}
          </p>
        </div>
        <button
          role="switch"
          aria-checked={desktopState.enabled}
          aria-label={t("settings_notifications_desktopEnable")}
          disabled={!canUseDesktop()}
          class="relative mt-0.5 inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors duration-200 {desktopState.enabled
            ? 'bg-primary'
            : 'bg-neutral-700'} {canUseDesktop() ? '' : 'cursor-not-allowed opacity-50'}"
          onclick={toggleDesktop}
        >
          <span
            class="inline-block h-4 w-4 transform rounded-full bg-white transition-transform {desktopState.enabled
              ? 'translate-x-6'
              : 'translate-x-1'}"
          ></span>
        </button>
      </div>

      <div class="flex flex-wrap items-center gap-3">
        <p
          role="status"
          class="text-xs {desktopState.permission === 'granted' && canUseDesktop()
            ? 'text-emerald-500'
            : 'text-muted-foreground'}"
        >
          {desktopStatusLabel()}
        </p>
        {#if canUseDesktop() && desktopState.permission === "default"}
          <Button variant="outline" size="sm" onclick={requestDesktopPermission}>
            {t("settings_notifications_desktopRequest")}
          </Button>
        {:else if canUseDesktop() && desktopState.permission === "granted"}
          <Button variant="outline" size="sm" onclick={runDesktopTest}>
            {t("settings_notifications_desktopTest")}
          </Button>
        {/if}
      </div>

      {#if desktopTestResult === "sent"}
        <p class="text-[11px] leading-snug text-muted-foreground">
          {t("settings_notifications_desktopTestSent")}
        </p>
      {:else if desktopTestResult === "unsupported" || desktopTestResult === "insecure"}
        <p class="text-[11px] leading-snug text-amber-400">
          {t("settings_notifications_desktopTestUnavailable")}
        </p>
      {/if}

      <p class="text-[11px] leading-snug text-muted-foreground">
        {t("settings_notifications_desktopHint")}
      </p>
      {#if !draft.enabled}
        <p class="rounded-md bg-muted/60 px-3 py-2 text-xs text-muted-foreground">
          {t("settings_notifications_desktopNeedsMaster")}
        </p>
      {/if}
    </Card>

    <!-- Master switch + events -->
    <Card class="p-4 sm:p-6 space-y-5">
      <div class="flex items-start justify-between gap-4">
        <div>
          <h2 class="text-sm font-semibold text-muted-foreground uppercase tracking-wider">
            {t("settings_notifications_title")}
          </h2>
          <p class="mt-1 text-xs text-muted-foreground">
            {t("settings_notifications_desc")}
          </p>
        </div>
        <button
          role="switch"
          aria-checked={draft.enabled}
          aria-label={t("settings_notifications_enable")}
          class="relative mt-0.5 inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors duration-200 {draft.enabled
            ? 'bg-primary'
            : 'bg-neutral-700'}"
          onclick={() => (draft.enabled = !draft.enabled)}
        >
          <span
            class="inline-block h-4 w-4 transform rounded-full bg-white transition-transform {draft.enabled
              ? 'translate-x-6'
              : 'translate-x-1'}"
          ></span>
        </button>
      </div>

      {#if !draft.enabled}
        <p class="rounded-md bg-muted/60 px-3 py-2 text-xs text-muted-foreground">
          {t("settings_notifications_masterOff")}
        </p>
      {/if}

      <div class="space-y-2">
        <p class="text-sm font-medium">{t("settings_notifications_events")}</p>
        <div class="grid gap-2 sm:grid-cols-3">
          {#each EVENT_OPTIONS as item (item.key)}
            <label
              class="flex cursor-pointer items-start gap-2 rounded-md border border-border px-3 py-2.5 transition-colors hover:bg-accent/40 {draft.enabled
                ? ''
                : 'opacity-60'}"
            >
              <input
                type="checkbox"
                class="mt-0.5 h-4 w-4 rounded border-input accent-primary"
                checked={draft[item.key as "on_complete" | "on_error" | "on_permission"]}
                onchange={(e) =>
                  (draft[item.key as "on_complete" | "on_error" | "on_permission"] =
                    e.currentTarget.checked)}
                disabled={!draft.enabled}
              />
              <span class="min-w-0">
                <span class="block text-xs font-medium">{t(item.label)}</span>
                <span class="mt-0.5 block text-[11px] leading-snug text-muted-foreground"
                  >{t(item.desc)}</span
                >
              </span>
            </label>
          {/each}
        </div>
      </div>

      <div class="grid gap-4 sm:grid-cols-2">
        <div>
          <label class="mb-1.5 block text-sm font-medium" for="notif-lang">
            {t("settings_notifications_language")}
          </label>
          <select
            id="notif-lang"
            bind:value={draft.language}
            class="h-10 w-full rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring sm:h-9"
          >
            <option value="zh-CN">{t("settings_notifications_langZh")}</option>
            <option value="en">{t("settings_notifications_langEn")}</option>
          </select>
        </div>
        <div>
          <label class="mb-1.5 block text-sm font-medium" for="notif-project-mode">
            {t("settings_notifications_projectPath")}
          </label>
          <select
            id="notif-project-mode"
            bind:value={draft.project_path_mode}
            class="h-10 w-full rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring sm:h-9"
          >
            <option value="basename">{t("settings_notifications_projectPathBasename")}</option>
            <option value="full">{t("settings_notifications_projectPathFull")}</option>
          </select>
          <p class="mt-1 text-[11px] text-muted-foreground">
            {t("settings_notifications_projectPathDesc")}
          </p>
        </div>
        <div class="sm:col-span-2">
          <label class="mb-1.5 block text-sm font-medium" for="notif-link-base">
            {t("settings_notifications_linkBase")}
          </label>
          <Input
            id="notif-link-base"
            bind:value={linkBaseText}
            placeholder="https://code.example.com"
          />
          <p class="mt-1 text-[11px] text-muted-foreground">
            {t("settings_notifications_linkBaseDesc")}
          </p>
        </div>
      </div>
    </Card>

    <!-- WeCom -->
    <Card class="p-4 sm:p-6 space-y-4">
      <div class="flex items-start justify-between gap-4">
        <div>
          <h3 class="text-sm font-semibold">{t("settings_notifications_wecom")}</h3>
          <p class="mt-0.5 text-xs text-muted-foreground">
            {t("settings_notifications_wecomDesc")}
          </p>
        </div>
        <button
          role="switch"
          aria-checked={draft.wecom.enabled}
          aria-label={t("settings_notifications_wecomEnable")}
          class="relative mt-0.5 inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors duration-200 {draft
            .wecom.enabled
            ? 'bg-primary'
            : 'bg-neutral-700'}"
          onclick={() => (draft.wecom.enabled = !draft.wecom.enabled)}
        >
          <span
            class="inline-block h-4 w-4 transform rounded-full bg-white transition-transform {draft
              .wecom.enabled
              ? 'translate-x-6'
              : 'translate-x-1'}"
          ></span>
        </button>
      </div>

      <div class="{draft.wecom.enabled ? '' : 'opacity-60'} space-y-4">
        <div>
          <label class="mb-1.5 block text-sm font-medium" for="notif-wecom-url">
            {t("settings_notifications_wecomWebhook")}
          </label>
          <Input
            id="notif-wecom-url"
            type="url"
            bind:value={draft.wecom.webhook_url}
            placeholder="https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=..."
          />
          <p class="mt-1 text-[11px] text-muted-foreground">
            {t("settings_notifications_wecomWebhookDesc")}
          </p>
        </div>
        <div>
          <label class="mb-1.5 block text-sm font-medium" for="notif-wecom-mobiles">
            {t("settings_notifications_wecomMobiles")}
          </label>
          <Input
            id="notif-wecom-mobiles"
            bind:value={mobilesText}
            placeholder="13800000000, 13900000000"
          />
          <p class="mt-1 text-[11px] text-muted-foreground">
            {t("settings_notifications_wecomMobilesDesc")}
          </p>
        </div>
      </div>

      <div class="flex flex-wrap items-center gap-3">
        <Button
          variant="outline"
          size="sm"
          loading={testChannel === "wecom"}
          disabled={testChannel !== null || !draft.wecom.webhook_url.trim()}
          onclick={() => runTest("wecom")}
        >
          {testChannel === "wecom"
            ? t("settings_notifications_testing")
            : t("settings_notifications_test")}
        </Button>
        {#if testResult?.channel === "wecom"}
          <p
            role="status"
            class="text-xs {testResult.result.ok ? 'text-emerald-500' : 'text-red-500'}"
          >
            {testResult.result.ok
              ? t("settings_notifications_testOk")
              : `${t("settings_notifications_testFailed")}: ${testResult.result.message}`}
          </p>
        {/if}
      </div>
    </Card>

    <!-- Custom HTTP -->
    <Card class="p-4 sm:p-6 space-y-4">
      <div class="flex items-start justify-between gap-4">
        <div>
          <h3 class="text-sm font-semibold">{t("settings_notifications_http")}</h3>
          <p class="mt-0.5 text-xs text-muted-foreground">
            {t("settings_notifications_httpDesc")}
          </p>
        </div>
        <button
          role="switch"
          aria-checked={draft.http.enabled}
          aria-label={t("settings_notifications_httpEnable")}
          class="relative mt-0.5 inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors duration-200 {draft
            .http.enabled
            ? 'bg-primary'
            : 'bg-neutral-700'}"
          onclick={() => (draft.http.enabled = !draft.http.enabled)}
        >
          <span
            class="inline-block h-4 w-4 transform rounded-full bg-white transition-transform {draft
              .http.enabled
              ? 'translate-x-6'
              : 'translate-x-1'}"
          ></span>
        </button>
      </div>

      <div class="{draft.http.enabled ? '' : 'opacity-60'} space-y-4">
        <div class="grid gap-3 sm:grid-cols-[1fr_120px]">
          <div>
            <label class="mb-1.5 block text-sm font-medium" for="notif-http-url">
              {t("settings_notifications_httpUrl")}
            </label>
            <Input
              id="notif-http-url"
              type="url"
              bind:value={draft.http.url}
              placeholder="https://example.com/hook"
            />
            <p class="mt-1 text-[11px] text-muted-foreground">
              {t("settings_notifications_httpUrlDesc")}
            </p>
          </div>
          <div>
            <label class="mb-1.5 block text-sm font-medium" for="notif-http-method">
              {t("settings_notifications_httpMethod")}
            </label>
            <select
              id="notif-http-method"
              bind:value={draft.http.method}
              class="h-10 w-full rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring sm:h-9"
            >
              <option value="POST">POST</option>
              <option value="GET">GET</option>
              <option value="PUT">PUT</option>
              <option value="PATCH">PATCH</option>
            </select>
          </div>
        </div>

        <div>
          <div class="mb-1.5 flex items-center justify-between gap-3">
            <span class="text-sm font-medium">{t("settings_notifications_httpHeaders")}</span>
            <Button variant="ghost" size="sm" onclick={addHeader}>
              + {t("settings_notifications_httpAddHeader")}
            </Button>
          </div>
          {#if (draft.http.headers ?? []).length === 0}
            <p class="text-[11px] text-muted-foreground">
              {t("settings_notifications_httpHeadersEmpty")}
            </p>
          {:else}
            <div class="space-y-2">
              {#each draft.http.headers ?? [] as header, index (index)}
                <div class="flex items-center gap-2">
                  <Input
                    bind:value={header.key}
                    placeholder={t("settings_notifications_httpHeaderKey")}
                    class="sm:max-w-[180px]"
                  />
                  <Input
                    bind:value={header.value}
                    placeholder={t("settings_notifications_httpHeaderValue")}
                  />
                  <button
                    type="button"
                    aria-label={t("settings_notifications_httpRemoveHeader")}
                    title={t("settings_notifications_httpRemoveHeader")}
                    class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                    onclick={() => removeHeader(index)}
                  >
                    ×
                  </button>
                </div>
              {/each}
            </div>
          {/if}
        </div>

        <div>
          <div class="mb-1.5 flex items-center justify-between gap-3">
            <label class="block text-sm font-medium" for="notif-http-body">
              {t("settings_notifications_httpBody")}
            </label>
            <Button variant="ghost" size="sm" onclick={fillBodyExample}>
              + {t("settings_notifications_httpBodyExample")}
            </Button>
          </div>
          <textarea
            id="notif-http-body"
            bind:value={draft.http.body_template}
            rows="5"
            spellcheck="false"
            placeholder={'{\n  "event": "{event}",\n  "title": "{title}",\n  "run_id": "{run_id}",\n  "message": "{message}"\n}'}
            class="w-full resize-y rounded-md border border-input bg-background px-3 py-2 font-mono text-xs text-foreground placeholder:text-muted-foreground/50 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          ></textarea>
          <p class="mt-1 text-[11px] text-muted-foreground">
            {t("settings_notifications_httpBodyDesc")}
          </p>
          {#if draft.http.method === "GET"}
            <p class="mt-1 text-[11px] text-amber-500">
              {t("settings_notifications_httpGetHint")}
            </p>
          {/if}
          <div class="mt-2 flex flex-wrap items-center gap-1.5">
            <span class="text-[11px] text-muted-foreground"
              >{t("settings_notifications_httpPlaceholders")}:</span
            >
            {#each PLACEHOLDERS as placeholder (placeholder)}
              <code class="rounded bg-muted px-1.5 py-0.5 text-[10px] text-foreground/80"
                >{placeholder}</code
              >
            {/each}
          </div>
        </div>
      </div>

      <div class="flex flex-wrap items-center gap-3">
        <Button
          variant="outline"
          size="sm"
          loading={testChannel === "http"}
          disabled={testChannel !== null || !draft.http.url.trim()}
          onclick={() => runTest("http")}
        >
          {testChannel === "http"
            ? t("settings_notifications_testing")
            : t("settings_notifications_test")}
        </Button>
        {#if testResult?.channel === "http"}
          <p
            role="status"
            class="text-xs {testResult.result.ok ? 'text-emerald-500' : 'text-red-500'}"
          >
            {testResult.result.ok
              ? t("settings_notifications_testOk")
              : `${t("settings_notifications_testFailed")}: ${testResult.result.message}`}
          </p>
        {/if}
      </div>
      <p class="text-[11px] text-muted-foreground">{t("settings_notifications_testHint")}</p>
    </Card>

    <!-- Save bar -->
    <div class="flex flex-wrap items-center gap-3">
      <Button loading={saving} disabled={saving} onclick={save}>
        {t("settings_notifications_save")}
      </Button>
      {#if saveMessage}
        <p role="status" class="text-xs text-emerald-500">{saveMessage}</p>
      {/if}
      {#if saveError}
        <p role="alert" class="text-xs text-red-500">{saveError}</p>
      {/if}
    </div>
  {/if}
</div>
