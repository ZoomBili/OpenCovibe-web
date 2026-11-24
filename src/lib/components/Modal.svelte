<script lang="ts">
  let {
    open = $bindable(false),
    title = "",
    closeable = true,
    children,
  }: {
    open?: boolean;
    title?: string;
    closeable?: boolean;
    children?: import("svelte").Snippet;
  } = $props();

  let dialogEl: HTMLDivElement | undefined = $state();

  // Auto-focus dialog container when opened so Escape hits onkeydown here
  $effect(() => {
    if (open) {
      dialogEl?.focus();
    }
  });

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      if (!closeable) {
        e.preventDefault();
        e.stopPropagation();
        return;
      }
      open = false;
    }
  }

  function handleBackdropClick() {
    if (!closeable) return;
    open = false;
  }
</script>

{#if open}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center p-3 sm:p-6"
    role="dialog"
    aria-modal="true"
    tabindex="-1"
    bind:this={dialogEl}
    onkeydown={handleKeydown}
  >
    <!-- Backdrop -->
    <div
      class="fixed inset-0 bg-black/60 backdrop-blur-sm"
      onclick={handleBackdropClick}
      role="presentation"
    ></div>

    <!-- Content: on phones this is a near-full-screen sheet so long forms stay usable. -->
    <div
      class="relative z-50 max-h-[calc(100dvh-1.5rem)] w-full overflow-y-auto rounded-xl border bg-background p-4 shadow-lg sm:max-h-[85dvh] sm:max-w-lg sm:rounded-lg sm:p-6"
    >
      {#if title}
        <h2 class="mb-4 text-base font-semibold sm:text-lg">{title}</h2>
      {/if}
      {#if children}
        {@render children()}
      {/if}
    </div>
  </div>
{/if}
