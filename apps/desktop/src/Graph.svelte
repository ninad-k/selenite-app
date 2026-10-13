<script>
  let { graph = { nodes: [], edges: [] }, selectedPath, onselect } = $props();
  let filter = $state('');
  const visible = $derived(graph.nodes.filter((node) => `${node.title} ${node.path}`.toLowerCase().includes(filter.toLowerCase())).slice(0, 120));
  const positioned = $derived(visible.map((node, index) => {
    const angle = (index / Math.max(visible.length, 1)) * Math.PI * 2 - Math.PI / 2;
    const radius = visible.length === 1 ? 0 : 195;
    return { ...node, x: 340 + Math.cos(angle) * radius * 1.25, y: 240 + Math.sin(angle) * radius };
  }));
  const byPath = $derived(new Map(positioned.map((node) => [node.path, node])));
  const edges = $derived(graph.edges.filter((edge) => byPath.has(edge.source) && byPath.has(edge.target)));
</script>

<section class="graph-panel">
  <div class="section-heading"><div><span class="eyebrow">CONNECTED THOUGHTS</span><h2>Your link graph</h2></div><span class="subtle">{graph.nodes.length} pages · {graph.edges.length} links</span></div>
  <label class="field-label">Find a page<input type="search" bind:value={filter} placeholder="Filter pages in the graph…" /></label>
  {#if !positioned.length}<p class="empty-state">No matching pages.</p>{:else}
    <svg class="vault-graph" viewBox="0 0 680 480" role="img" aria-label="Vault page connections">
      {#each edges as edge}
        <line x1={byPath.get(edge.source).x} y1={byPath.get(edge.source).y} x2={byPath.get(edge.target).x} y2={byPath.get(edge.target).y} stroke="#d0dcc7" stroke-width="1" />
      {/each}
      {#each positioned as node (node.path)}
        <a href={`#page=${encodeURIComponent(node.path)}`} aria-label={`Open ${node.title}`} onclick={(event) => { event.preventDefault(); onselect(node.path); }} class="graph-node" class:current={node.path === selectedPath}>
          <circle cx={node.x} cy={node.y} r={node.path === selectedPath ? 7 : 5} fill={node.path === selectedPath ? '#3f6850' : '#8aaa7a'}><title>{node.path}</title></circle>
          {#if positioned.length <= 30}<text x={node.x} y={node.y - 13} text-anchor="middle" fill="#64785d" font-size="10">{node.title.length > 23 ? `${node.title.slice(0, 20)}…` : node.title}</text>{/if}
        </a>
      {/each}
    </svg>
    {#if graph.nodes.length > 120}<p class="subtle">Showing the first 120 matching pages. Filter to explore a smaller set.</p>{/if}
    <div class="graph-page-list">{#each positioned as node (node.path)}<button class="text-button" onclick={() => onselect(node.path)} title={node.path}>{node.title}</button>{/each}</div>
  {/if}
</section>
