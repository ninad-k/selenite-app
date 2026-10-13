<script>
  import { onMount } from 'svelte';
  let { label, oncancel, disabled = false, children } = $props();
  let dialog;
  onMount(() => {
    dialog.showModal();
    return () => dialog.close();
  });
</script>

<dialog class="dialog" bind:this={dialog} aria-label={label} oncancel={(event) => { event.preventDefault(); if (!disabled) oncancel?.(); }}>
  {@render children()}
</dialog>
