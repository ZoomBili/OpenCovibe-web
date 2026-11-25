<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { goto } from "$app/navigation";
  import * as api from "$lib/api";
  import type {
    AutomationTask,
    AutomationTaskInput,
    CronValidation,
    StandaloneSkill,
  } from "$lib/types";
  import { t } from "$lib/i18n/index.svelte";
  import { dbg, dbgWarn } from "$lib/utils/debug";
  import Card from "$lib/components/Card.svelte";
  import Button from "$lib/components/Button.svelte";
  import Input from "$lib/components/Input.svelte";

  interface SkillOption {
    key: string;
    name: string;
    path: string;
    agent: "claude" | "codex";
    description: string;
    scope?: string;
  }

  interface AutomationDraft {
    name: string;
    skill_name: string;
    skill_path: string;
    skill_agent: "claude" | "codex";
    prompt: string;
    cwd: string;
    model: string;
    cron: string;
    enabled: boolean;
  }

  function emptyDraft(): AutomationDraft {
    return {
      name: "",
      skill_name: "",
      skill_path: "",
      skill_agent: "claude",
      prompt: "",
      cwd: "",
      model: "",
      cron: "0 9 * * *",
      enabled: true,
    };
  }

  type MessageKey = Parameters<typeof t>[0];

  const CRON_PRESETS: { labelKey: MessageKey; expression: string }[] = [
    { labelKey: "settings_automation_presetEvery5Min", expression: "*/5 * * * *" },
    { labelKey: "settings_automation_presetEveryHour", expression: "0 * * * *" },
    { labelKey: "settings_automation_presetEveryDay9", expression: "0 9 * * *" },
    { labelKey: "settings_automation_presetEveryMonday9", expression: "0 9 * * 1" },
  ];

  let tasks = $state<AutomationTask[]>([]);
  let skills = $state<SkillOption[]>([]);
  /** Saved skill from the task being edited that isn't in the current project's list. */
  let fallbackSkill = $state<SkillOption | null>(null);
  let loading = $state(true);
  let skillsLoading = $state(true);
  let error = $state("");
  let notice = $state("");
  let busyId = $state<string | null>(null);
  let saving = $state(false);
  let expandedLogs = $state<Record<string, boolean>>({});

  let formOpen = $state(false);
  let editingId = $state<string | null>(null);
  let draft = $state<AutomationDraft>(emptyDraft());
  let selectedSkillKey = $state("");
  let cronValidation = $state<CronValidation | null>(null);
  let cronChecking = $state(false);
  let cronTimer: ReturnType<typeof setTimeout> | null = null;
  let refreshTimer: ReturnType<typeof setInterval> | null = null;

  let availableSkills = $derived.by(() => {
    const list = [...skills];
    const extra = fallbackSkill;
    if (extra && !list.some((skill) => skill.key === extra.key)) list.push(extra);
    return list;
  });

  let skillGroups = $derived.by(() =>
    (
      [
        {
          id: "claude",
          label: "Claude",
          items: availableSkills.filter((s) => s.agent === "claude"),
        },
        {
          id: "codex",
          label: "Codex",
          items: availableSkills.filter((s) => s.agent === "codex"),
        },
      ] as { id: string; label: string; items: SkillOption[] }[]
    ).filter((group) => group.items.length > 0),
  );

  let nextRunsText = $derived((cronValidation?.next_runs_local ?? []).filter(Boolean).join(" · "));

  onMount(async () => {
    await Promise.all([loadTasks(), loadSkills()]);
    // Keep "next run" and in-progress execution status fresh while the tab is open.
    refreshTimer = setInterval(() => void loadTasks(), 30_000);
  });

  onDestroy(() => {
    if (cronTimer) clearTimeout(cronTimer);
    if (refreshTimer) clearInterval(refreshTimer);
  });

  function errText(value: unknown): string {
    if (typeof value === "string") return value;
    if (value instanceof Error) return value.message;
    return String(value);
  }

  function formatTime(iso?: string | null): string {
    if (!iso) return "";
    const date = new Date(iso);
    if (Number.isNaN(date.getTime())) return iso;
    return date.toLocaleString();
  }

  async function loadTasks() {
    try {
      tasks = await api.listAutomations();
      error = "";
    } catch (e) {
      dbgWarn("automation", "load tasks failed", e);
      error = t("settings_automation_loadFailed", { error: errText(e) });
    } finally {
      loading = false;
    }
  }

  async function loadSkills() {
    skillsLoading = true;
    try {
      const cwd = localStorage.getItem("ocv:project-cwd") || undefined;
      const [claude, codex] = await Promise.allSettled([
        api.listStandaloneSkills(cwd),
        api.listCodexSkills(cwd),
      ]);

      const options: SkillOption[] = [];
      const push = (list: StandaloneSkill[], agent: "claude" | "codex") => {
        for (const skill of list) {
          if (skill.enabled === false || !skill.name || !skill.path) continue;
          options.push({
            key: `${agent}:${skill.path}`,
            name: skill.name,
            path: skill.path,
            agent,
            description: skill.description ?? "",
            scope: skill.scope,
          });
        }
      };

      if (claude.status === "fulfilled") push(claude.value, "claude");
      else dbgWarn("automation", "claude skills load failed", claude.reason);
      if (codex.status === "fulfilled") push(codex.value, "codex");
      else dbgWarn("automation", "codex skills load failed", codex.reason);

      options.sort((a, b) => a.agent.localeCompare(b.agent) || a.name.localeCompare(b.name));
      skills = options;
      dbg("automation", "skills loaded", options.length);
    } finally {
      skillsLoading = false;
    }
  }

  function openCreate() {
    draft = emptyDraft();
    selectedSkillKey = "";
    fallbackSkill = null;
    editingId = null;
    formOpen = true;
    error = "";
    notice = "";
    scheduleCronCheck();
  }

  function openEdit(task: AutomationTask) {
    draft = {
      name: task.name,
      skill_name: task.skill_name,
      skill_path: task.skill_path,
      skill_agent: task.skill_agent,
      prompt: task.prompt,
      cwd: task.cwd,
      model: task.model ?? "",
      cron: task.cron,
      enabled: task.enabled,
    };
    const match = skills.find(
      (skill) =>
        skill.agent === task.skill_agent &&
        skill.name === task.skill_name &&
        (!task.skill_path || skill.path === task.skill_path),
    );
    // The saved skill may belong to another project, so it won't be in the list loaded
    // for the current project. Keep it selectable instead of showing an empty "choose".
    fallbackSkill =
      !match && task.skill_name
        ? {
            key: `saved:${task.skill_agent}:${task.skill_path || task.skill_name}`,
            name: task.skill_name,
            path: task.skill_path,
            agent: task.skill_agent,
            description: "",
            scope:
              task.skill_path.includes("/.claude/") ||
              task.skill_path.includes("/.agents/") ||
              task.skill_path.includes("/.codex/")
                ? "project"
                : undefined,
          }
        : null;
    selectedSkillKey = match?.key ?? fallbackSkill?.key ?? "";
    editingId = task.id;
    formOpen = true;
    error = "";
    notice = "";
    scheduleCronCheck();
  }

  function closeForm() {
    formOpen = false;
    editingId = null;
    fallbackSkill = null;
    if (cronTimer) {
      clearTimeout(cronTimer);
      cronTimer = null;
    }
    cronChecking = false;
  }

  /** Project root encoded in a project-scoped skill path, if any. */
  function projectRootFromSkillPath(path: string): string | null {
    const normalized = path.replace(/\\/g, "/");
    for (const marker of ["/.claude/", "/.agents/", "/.codex/"]) {
      const index = normalized.indexOf(marker);
      if (index > 0) return normalized.slice(0, index);
    }
    return null;
  }

  function onSkillChange(event: Event) {
    const key = (event.currentTarget as HTMLSelectElement).value;
    selectedSkillKey = key;
    const skill = availableSkills.find((candidate) => candidate.key === key);
    if (!skill) return;
    draft.skill_name = skill.name;
    draft.skill_path = skill.path;
    draft.skill_agent = skill.agent;
    if (!draft.name.trim()) draft.name = skill.name;
    // Project skills are only discovered when Claude runs in that project directory.
    if (!draft.cwd.trim() && skill.scope === "project") {
      const projectRoot = projectRootFromSkillPath(skill.path);
      if (projectRoot) draft.cwd = projectRoot;
    }
  }

  function scheduleCronCheck() {
    if (cronTimer) clearTimeout(cronTimer);
    const expression = draft.cron.trim();
    if (!expression) {
      cronValidation = null;
      cronChecking = false;
      return;
    }
    cronChecking = true;
    cronTimer = setTimeout(async () => {
      try {
        cronValidation = await api.validateCron(expression);
      } catch (e) {
        cronValidation = { valid: false, error: errText(e) };
      } finally {
        cronChecking = false;
      }
    }, 300);
  }

  function applyPreset(expression: string) {
    draft.cron = expression;
    scheduleCronCheck();
  }

  function toInput(value: AutomationDraft): AutomationTaskInput {
    return {
      name: value.name.trim() || value.skill_name.trim(),
      skill_name: value.skill_name.trim(),
      skill_path: value.skill_path.trim(),
      skill_agent: value.skill_agent,
      prompt: value.prompt.trim(),
      cwd: value.cwd.trim(),
      model: value.model.trim() || null,
      cron: value.cron.trim(),
      enabled: value.enabled,
    };
  }

  function inputFromTask(task: AutomationTask, enabled: boolean): AutomationTaskInput {
    return {
      name: task.name,
      skill_name: task.skill_name,
      skill_path: task.skill_path,
      skill_agent: task.skill_agent,
      prompt: task.prompt,
      cwd: task.cwd,
      model: task.model ?? null,
      cron: task.cron,
      enabled,
    };
  }

  async function save() {
    error = "";
    notice = "";
    const input = toInput(draft);
    if (!input.skill_name) {
      error = t("settings_automation_skillRequired");
      return;
    }
    if (!input.cron) {
      error = t("settings_automation_cronRequired");
      return;
    }

    saving = true;
    try {
      const validation = await api.validateCron(input.cron);
      cronValidation = validation;
      if (!validation.valid) {
        error = t("settings_automation_cronInvalid", { error: validation.error ?? "" });
        return;
      }
      if (editingId) await api.updateAutomation(editingId, input);
      else await api.createAutomation(input);
      formOpen = false;
      editingId = null;
      notice = t("settings_automation_saved");
      await loadTasks();
    } catch (e) {
      dbgWarn("automation", "save failed", e);
      error = t("settings_automation_saveFailed", { error: errText(e) });
    } finally {
      saving = false;
    }
  }

  async function toggleTask(task: AutomationTask) {
    busyId = task.id;
    error = "";
    try {
      await api.updateAutomation(task.id, inputFromTask(task, !task.enabled));
      await loadTasks();
    } catch (e) {
      error = t("settings_automation_actionFailed", { error: errText(e) });
    } finally {
      busyId = null;
    }
  }

  async function runNow(task: AutomationTask) {
    busyId = task.id;
    error = "";
    notice = "";
    try {
      const result = await api.runAutomationNow(task.id);
      notice = t("settings_automation_runStarted", { runId: result.run_id });
      await loadTasks();
    } catch (e) {
      error = t("settings_automation_actionFailed", { error: errText(e) });
    } finally {
      busyId = null;
    }
  }

  async function removeTask(task: AutomationTask) {
    if (!confirm(t("settings_automation_deleteConfirm", { name: task.name }))) return;
    busyId = task.id;
    error = "";
    try {
      await api.deleteAutomation(task.id);
      await loadTasks();
    } catch (e) {
      error = t("settings_automation_actionFailed", { error: errText(e) });
    } finally {
      busyId = null;
    }
  }

  function statusLabel(status?: string | null): string {
    switch (status) {
      case "running":
        return t("settings_automation_status_running");
      case "idle":
        return t("settings_automation_status_idle");
      case "completed":
        return t("settings_automation_status_completed");
      case "failed":
        return t("settings_automation_status_failed");
      case "stopped":
        return t("settings_automation_status_stopped");
      case "timeout":
        return t("settings_automation_status_timeout");
      default:
        return status ?? "";
    }
  }

  function statusClass(status: string): string {
    switch (status) {
      case "completed":
        return "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400";
      case "failed":
      case "timeout":
        return "bg-destructive/10 text-destructive";
      case "running":
        return "bg-primary/10 text-primary";
      default:
        return "bg-muted text-muted-foreground";
    }
  }

  function durationLabel(startedAt?: string | null, finishedAt?: string | null): string {
    if (!startedAt) return "";
    const start = new Date(startedAt).getTime();
    const end = finishedAt ? new Date(finishedAt).getTime() : Date.now();
    if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) return "";
    const seconds = Math.max(0, Math.round((end - start) / 1000));
    if (seconds < 60) return `${seconds}s`;
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
    return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
  }

  function toggleLogs(id: string) {
    expandedLogs[id] = !expandedLogs[id];
  }

  function openSession(runId?: string | null) {
    if (!runId) return;
    goto(`/chat?run=${encodeURIComponent(runId)}`);
  }
</script>

<div class="space-y-5">
  <Card class="p-4 sm:p-6 space-y-3">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 class="text-sm font-semibold text-muted-foreground uppercase tracking-wider">
          {t("settings_automation_title")}
        </h2>
        <p class="mt-1 text-xs text-muted-foreground">
          {t("settings_automation_description")}
        </p>
      </div>
      <Button size="sm" onclick={openCreate}>{t("settings_automation_new")}</Button>
    </div>
    {#if notice}
      <p class="text-xs text-emerald-600 dark:text-emerald-400">{notice}</p>
    {/if}
    {#if error}
      <p class="text-xs text-destructive">{error}</p>
    {/if}
  </Card>

  {#if formOpen}
    <Card class="p-4 sm:p-6 space-y-4">
      <div class="flex items-center justify-between">
        <h3 class="text-sm font-semibold">
          {editingId ? t("settings_automation_editTitle") : t("settings_automation_newTitle")}
        </h3>
        <Button variant="ghost" size="sm" onclick={closeForm}>{t("common_cancel")}</Button>
      </div>

      <div class="grid gap-4 sm:grid-cols-2">
        <div class="space-y-1.5">
          <label class="text-sm font-medium" for="automation-name">
            {t("settings_automation_name")}
          </label>
          <Input
            id="automation-name"
            bind:value={draft.name}
            placeholder={t("settings_automation_namePlaceholder")}
          />
        </div>

        <div class="space-y-1.5">
          <label class="text-sm font-medium" for="automation-skill">
            {t("settings_automation_skill")}
          </label>
          {#if skillsLoading}
            <p class="text-xs text-muted-foreground">{t("settings_automation_skillsLoading")}</p>
          {:else if skills.length === 0}
            <p class="text-xs text-muted-foreground">{t("settings_automation_noSkills")}</p>
          {:else}
            <select
              id="automation-skill"
              class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring sm:h-9"
              value={selectedSkillKey}
              onchange={onSkillChange}
            >
              <option value="">{t("settings_automation_chooseSkill")}</option>
              {#each skillGroups as group (group.id)}
                <optgroup label={group.label}>
                  {#each group.items as skill (skill.key)}
                    <option value={skill.key}>
                      {skill.name}{skill.scope ? ` · ${skill.scope}` : ""}
                    </option>
                  {/each}
                </optgroup>
              {/each}
            </select>
          {/if}
        </div>
      </div>

      <div class="space-y-1.5">
        <label class="text-sm font-medium" for="automation-cron">
          {t("settings_automation_cron")}
        </label>
        <div class="flex flex-wrap items-center gap-2">
          <Input
            id="automation-cron"
            class="sm:max-w-xs"
            bind:value={draft.cron}
            placeholder={t("settings_automation_cronPlaceholder")}
            oninput={scheduleCronCheck}
          />
          {#each CRON_PRESETS as preset (preset.expression)}
            <button
              type="button"
              class="rounded-md border px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              onclick={() => applyPreset(preset.expression)}
            >
              {t(preset.labelKey)}
            </button>
          {/each}
        </div>
        <p class="text-xs text-muted-foreground">
          {t("settings_automation_cronHelp")}
        </p>
        {#if cronChecking}
          <p class="text-xs text-muted-foreground">{t("settings_automation_cronChecking")}</p>
        {:else if cronValidation?.valid}
          <p class="text-xs text-emerald-600 dark:text-emerald-400">
            {t("settings_automation_cronValid", { runs: nextRunsText })}
            {#if cronValidation.timezone}
              <span class="ml-1 opacity-70">({cronValidation.timezone})</span>
            {/if}
          </p>
        {:else if cronValidation && !cronValidation.valid}
          <p class="text-xs text-destructive">
            {t("settings_automation_cronInvalid", { error: cronValidation.error ?? "" })}
          </p>
        {/if}
      </div>

      <div class="space-y-1.5">
        <label class="text-sm font-medium" for="automation-prompt">
          {t("settings_automation_prompt")}
        </label>
        <textarea
          id="automation-prompt"
          class="min-h-20 w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          bind:value={draft.prompt}
          placeholder={t("settings_automation_promptPlaceholder")}
        ></textarea>
      </div>

      <div class="grid gap-4 sm:grid-cols-2">
        <div class="space-y-1.5">
          <label class="text-sm font-medium" for="automation-cwd">
            {t("settings_automation_cwd")}
          </label>
          <Input
            id="automation-cwd"
            bind:value={draft.cwd}
            placeholder={t("settings_automation_cwdPlaceholder")}
          />
        </div>
        <div class="space-y-1.5">
          <label class="text-sm font-medium" for="automation-model">
            {t("settings_automation_model")}
          </label>
          <Input
            id="automation-model"
            bind:value={draft.model}
            placeholder={t("settings_automation_modelPlaceholder")}
          />
        </div>
      </div>

      <label class="flex items-center gap-2 text-sm">
        <input type="checkbox" bind:checked={draft.enabled} class="h-4 w-4 rounded border-input" />
        {t("settings_automation_enabled")}
      </label>

      <div class="flex justify-end gap-2 border-t border-border pt-4">
        <Button variant="outline" size="sm" onclick={closeForm}>{t("common_cancel")}</Button>
        <Button size="sm" loading={saving} disabled={saving} onclick={save}>
          {editingId ? t("common_save") : t("settings_automation_create")}
        </Button>
      </div>
    </Card>
  {/if}

  {#if loading}
    <p class="text-sm text-muted-foreground">{t("settings_automation_loading")}</p>
  {:else if tasks.length === 0}
    <Card class="p-6 text-center">
      <p class="text-sm font-medium">{t("settings_automation_empty")}</p>
      <p class="mt-1 text-xs text-muted-foreground">{t("settings_automation_emptyHint")}</p>
    </Card>
  {:else}
    <div class="space-y-3">
      {#each tasks as task (task.id)}
        <Card class="p-4 space-y-3">
          <div class="flex flex-wrap items-start justify-between gap-3">
            <div class="min-w-0 space-y-1">
              <div class="flex flex-wrap items-center gap-2">
                <span class="truncate text-sm font-medium">{task.name}</span>
                <span
                  class="rounded-full bg-muted px-2 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground"
                >
                  {task.skill_agent}
                </span>
                <span
                  class="rounded-full px-2 py-0.5 text-[10px] font-medium {task.enabled
                    ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400'
                    : 'bg-muted text-muted-foreground'}"
                >
                  {task.enabled
                    ? t("settings_automation_status_enabled")
                    : t("settings_automation_status_disabled")}
                </span>
              </div>
              <p class="text-xs text-muted-foreground">
                <code class="rounded bg-muted px-1 py-0.5">{task.cron}</code>
                <span class="mx-1">·</span>
                /{task.skill_name}
                {#if task.cwd}<span class="mx-1">·</span>{task.cwd}{/if}
              </p>
              {#if task.enabled && task.next_run_local}
                <p class="text-xs text-muted-foreground">
                  {t("settings_automation_nextRun", { time: task.next_run_local })}
                  <span class="ml-1 opacity-70">
                    ({t("settings_automation_serverTime")})
                  </span>
                </p>
              {:else if !task.enabled}
                <p class="text-xs text-muted-foreground">
                  {t("settings_automation_nextRunDisabled")}
                </p>
              {/if}
              <p class="text-xs text-muted-foreground">
                {#if task.last_run_at}
                  {t("settings_automation_lastRun", { time: formatTime(task.last_run_at) })}
                  {#if task.last_status}
                    <span class="mx-1">·</span>{statusLabel(task.last_status)}
                  {/if}
                {:else}
                  {t("settings_automation_neverRun")}
                {/if}
              </p>
              {#if task.executions?.[0]?.result_preview}
                <p class="line-clamp-2 whitespace-pre-wrap break-words text-xs text-foreground/80">
                  {task.executions[0].result_preview}
                </p>
              {/if}
              {#if task.last_error}
                <p class="text-xs text-destructive">
                  {t("settings_automation_lastError", { error: task.last_error })}
                </p>
              {/if}
            </div>

            <div class="flex flex-wrap items-center gap-1.5">
              {#if task.last_run_id}
                <Button variant="ghost" size="sm" onclick={() => openSession(task.last_run_id)}>
                  {t("settings_automation_viewSession")}
                </Button>
              {/if}
              <Button
                variant="outline"
                size="sm"
                disabled={busyId === task.id}
                onclick={() => toggleTask(task)}
              >
                {task.enabled ? t("settings_automation_disable") : t("settings_automation_enable")}
              </Button>
              <Button
                variant="outline"
                size="sm"
                loading={busyId === task.id}
                onclick={() => runNow(task)}
              >
                {t("settings_automation_runNow")}
              </Button>
              <Button variant="ghost" size="sm" onclick={() => openEdit(task)}>
                {t("common_edit")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                class="text-destructive"
                disabled={busyId === task.id}
                onclick={() => removeTask(task)}
              >
                {t("settings_automation_delete")}
              </Button>
            </div>
          </div>

          <div class="border-t border-border pt-3">
            <button
              type="button"
              class="flex w-full items-center justify-between text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
              onclick={() => toggleLogs(task.id)}
            >
              <span>
                {t("settings_automation_logsCount", {
                  count: String(task.executions?.length ?? 0),
                })}
              </span>
              <svg
                class="h-3 w-3 transition-transform {expandedLogs[task.id] ? 'rotate-180' : ''}"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="m6 9 6 6 6-6" />
              </svg>
            </button>

            {#if expandedLogs[task.id]}
              {#if !task.executions || task.executions.length === 0}
                <p class="mt-2 text-xs text-muted-foreground">
                  {t("settings_automation_noLogs")}
                </p>
              {:else}
                <ul class="mt-2 space-y-2">
                  {#each task.executions as execution (execution.id || execution.run_id || execution.started_at)}
                    <li class="space-y-1.5 rounded-md border border-border/70 bg-muted/30 p-2.5">
                      <div class="flex flex-wrap items-center gap-2 text-xs">
                        <span
                          class="rounded-full px-2 py-0.5 text-[10px] font-medium {statusClass(
                            execution.status,
                          )}"
                        >
                          {statusLabel(execution.status)}
                        </span>
                        {#if execution.run_id}
                          <span class="text-muted-foreground">
                            {formatTime(execution.started_at)}
                          </span>
                        {:else}
                          <span class="text-muted-foreground">
                            {t("settings_automation_attemptFailed")}
                          </span>
                        {/if}
                        {#if execution.finished_at}
                          <span class="text-muted-foreground">
                            · {durationLabel(execution.started_at, execution.finished_at)}
                          </span>
                        {:else}
                          <span class="text-muted-foreground">
                            · {t("settings_automation_logRunning")}
                          </span>
                        {/if}
                        {#if execution.result_subtype}
                          <span class="text-muted-foreground">· {execution.result_subtype}</span>
                        {/if}
                      </div>

                      {#if execution.result_preview}
                        <p
                          class="line-clamp-4 whitespace-pre-wrap break-words text-xs text-foreground/90"
                        >
                          {execution.result_preview}
                        </p>
                      {/if}
                      {#if execution.error}
                        <p class="whitespace-pre-wrap break-words text-xs text-destructive">
                          {execution.error}
                        </p>
                      {/if}
                      {#if execution.run_id}
                        <Button
                          variant="ghost"
                          size="sm"
                          class="h-6 px-2 text-xs"
                          onclick={() => openSession(execution.run_id)}
                        >
                          {t("settings_automation_viewSession")}
                        </Button>
                      {/if}
                    </li>
                  {/each}
                </ul>
              {/if}
            {/if}
          </div>
        </Card>
      {/each}
    </div>
  {/if}
</div>
