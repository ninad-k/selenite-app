<script>
  import Icon from './Icon.svelte';
  import TreeNode from './TreeNode.svelte';

  let { node, selectedPath, onselect, disabled = false, depth = 0 } = $props();
  let expanded = $state(false);

  $effect(() => {
    // Opening Today changes the busy state even when its path is already selected.
    // Track that transition so its folder is revealed again after manual collapse.
    void disabled;
    if (selectedPath?.startsWith(`${node.path}/`)) expanded = true;
  });
</script>

<li class="tree-node">
  {#if node.kind === 'folder'}
    <button
      class="tree-row folder-row"
      style:--depth={depth}
      aria-expanded={expanded}
      onclick={() => (expanded = !expanded)}
      title={node.path}
    >
      <Icon name="chevron" size={13} class={expanded ? 'chevron expanded' : 'chevron'} />
      <Icon name={expanded ? 'folder-open' : 'folder'} size={16} />
      <span class="tree-name">{node.name}</span>
    </button>
    {#if expanded}
      <ul class="tree-children">
        {#each node.children as child (child.path)}
          <TreeNode node={child} {selectedPath} {onselect} {disabled} depth={depth + 1} />
        {/each}
      </ul>
    {/if}
  {:else}
    <button
      class="tree-row file-row"
      class:selected={selectedPath === node.path}
      style:--depth={depth}
      aria-current={selectedPath === node.path ? 'page' : undefined}
      title={node.path}
      {disabled}
      onclick={() => onselect(node.path)}
    >
      <span class="tree-spacer"></span>
      <Icon name="note" size={16} />
      <span class="tree-name">{node.name.replace(/\.md$/i, '')}</span>
    </button>
  {/if}
</li>
