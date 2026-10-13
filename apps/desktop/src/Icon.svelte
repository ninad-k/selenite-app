<script>
  let { name, size = 18, class: className = '' } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.6"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  class={className}
>
  {#if name === 'crystal'}
    <path d="m12 2 7 5v10l-7 5-7-5V7l7-5Z" />
    <path d="m12 2-3 6v8l3 6 3-6V8l-3-6Zm-7 5 4 1m6 0 4-1M5 17l4-1m6 0 4 1" />
  {:else if name === 'folder'}
    <path d="M3 7V5a1 1 0 0 1 1-1h5l2 3h9a1 1 0 0 1 1 1v11a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7Z" />
  {:else if name === 'folder-open'}
    <path d="M3 17V5a1 1 0 0 1 1-1h5l2 3h8a1 1 0 0 1 1 1v2" />
    <path d="M4 10h17l-3 10H1l3-10Z" />
  {:else if name === 'note'}
    <path d="M14 3H5v18h14V8l-5-5Z" />
    <path d="M14 3v5h5M8 12h8m-8 4h6" />
  {:else if name === 'calendar'}
    <rect x="3" y="5" width="18" height="16" rx="2" />
    <path d="M7 3v4m10-4v4M3 10h18m-13 5h3" />
  {:else if name === 'chevron'}
    <path d="m9 6 6 6-6 6" />
  {:else if name === 'arrow'}
    <path d="M5 12h14m-6-6 6 6-6 6" />
  {:else if name === 'check'}
    <path d="m5 12 4 4L19 6" />
  {:else if name === 'warning'}
    <path d="m12 3 10 18H2L12 3Zm0 6v5m0 3h.01" />
  {:else if name === 'close'}
    <path d="m6 6 12 12M6 18 18 6" />
  {/if}
</svg>
