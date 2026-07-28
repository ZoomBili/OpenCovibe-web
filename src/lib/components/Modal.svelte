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
    class="fixed inset-0 z-50 flex items-center justify-center"
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

    <!-- Content -->
    <div class="relative z-50 w-full max-w-lg rounded-lg border bg-background p-6 shadow-lg">
      {#if title}
        <h2 class="mb-4 text-lg font-semibold">{title}</h2>
      {/if}
      {#if children}
        {@render children()}
      {/if}
    </div>
  </div>
{/if}
