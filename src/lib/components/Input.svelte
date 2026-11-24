<script lang="ts">
  let {
    value = $bindable(""),
    placeholder = "",
    type = "text",
    id = undefined,
    disabled = false,
    class: className = "",
    oninput,
    onblur,
    onpaste,
  }: {
    value?: string;
    placeholder?: string;
    type?: string;
    id?: string;
    disabled?: boolean;
    class?: string;
    oninput?: (e: Event) => void;
    onblur?: (e: FocusEvent) => void;
    onpaste?: (e: ClipboardEvent) => void;
  } = $props();
</script>

<input
  {id}
  {type}
  {placeholder}
  {disabled}
  bind:value
  {oninput}
  {onblur}
  {onpaste}
  class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm sm:h-9 transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 {className}"
/>
