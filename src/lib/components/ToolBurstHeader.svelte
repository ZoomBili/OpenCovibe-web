<script lang="ts">
  import type { ToolBurst } from "$lib/utils/tool-rendering";
  import { t } from "$lib/i18n/index.svelte";

  let {
    burst,
    collapsed,
    onToggle,
  }: {
    burst: ToolBurst;
    collapsed: boolean;
    onToggle: () => void;
  } = $props();

  let allDone = $derived(
    burst.stats.total > 0 && burst.stats.completed + burst.stats.failed === burst.stats.total,
  );

  let summaryText = $derived(burst.summary.map((s) => `${s.count}\u00d7 ${s.toolName}`).join(", "));

  let ariaLabel = $derived(
    `${burst.stats.total} tool calls: ${summaryText}. ${burst.stats.completed} of ${burst.stats.total} completed.${collapsed ? " Collapsed." : " Expanded."}`,
  );
</script>

<button
  class="w-full rounded-lg border border-border/40 bg-muted/20 px-3 py-2 text-left transition-colors hover:bg-muted/40"
  aria-expanded={!collapsed}
  aria-label={ariaLabel}
  onclick={onToggle}
>
  <div class="flex items-center gap-1.5 text-xs text-muted-foreground">
    <!-- Collapse arrow -->
    <span class="inline-block w-3 shrink-0 text-center text-[10px]">
      {collapsed ? "\u25b8" : "\u25be"}
    </span>

    <!-- Status indicator -->
    {#if allDone}
      <svg
        class="h-3 w-3 text-emerald-500 shrink-0"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2.5"
        stroke-linecap="round"
        stroke-linejoin="round"><path d="M20 6 9 17l-5-5" /></svg
      >
    {:else if burst.stats.running > 0}
      <span class="inline-block h-1.5 w-1.5 rounded-full bg-blue-400 animate-pulse shrink-0"></span>
    {/if}

    <!-- Summary -->
    <span>
      {t("toolBurst_calls", { total: String(burst.stats.total) })} &mdash; {summaryText}
    </span>

    <!-- Stats -->
    <span class="ml-auto tabular-nums whitespace-nowrap">
      {t("toolBurst_completed", {
        completed: String(burst.stats.completed),
        total: String(burst.stats.total),
      })}
    </span>
    {#if burst.stats.failed > 0}
      <span class="text-destructive tabular-nums">
        {t("toolBurst_failed", { failed: String(burst.stats.failed) })}
      </span>
    {/if}
  </div>

  <!-- Progress bar -->
  <div class="mt-1.5 h-1.5 rounded-full bg-border/30 overflow-hidden flex">
    {#if burst.stats.completed > 0}
      <div
        class="bg-emerald-500 transition-all duration-300"
        style:width="{(burst.stats.completed / burst.stats.total) * 100}%"
      ></div>
    {/if}
    {#if burst.stats.failed > 0}
      <div
        class="bg-destructive transition-all duration-300"
        style:width="{(burst.stats.failed / burst.stats.total) * 100}%"
      ></div>
    {/if}
  </div>
</button>
