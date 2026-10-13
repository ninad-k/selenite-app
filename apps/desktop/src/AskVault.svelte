<script>
  import { invoke } from '@tauri-apps/api/core';
  import Dialog from './Dialog.svelte';
  let { onselect, onsettings } = $props();
  let question = $state('');
  let answer = $state(null);
  let busy = $state(false);
  let error = $state('');
  let notice = $state('');
  let confirmation = $state(null);
  let acknowledged = $state(false);
  let reviewedConfig = $state('');
  let reviewedQuestion = $state('');
  async function perform() {
    const action = confirmation;
    confirmation = null; busy = true; error = ''; notice = '';
    try {
      if (action === 'index') {
        const result = await invoke('ai_index', { expectedConfig: reviewedConfig });
        notice = `Built vectors for ${result.pages} pages and ${result.chunks} passages (${result.dimensions} dimensions).`;
      } else answer = await invoke('ask_vault', { question: reviewedQuestion, expectedConfig: reviewedConfig });
    } catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
  async function confirm(action) {
    acknowledged = false; busy = true; error = '';
    try {
      const config = await invoke('read_service_config', { service: 'ai' });
      if (!config.source) throw new Error('Configure an AI provider in Services & settings first.');
      reviewedConfig = config.source; reviewedQuestion = question; confirmation = action;
    } catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
</script>

<section class="ask-panel" data-testid="ask-vault">
  <div class="section-heading"><div><span class="eyebrow">FIND THE THREAD</span><h2>Ask your vault</h2></div><button class="text-button" onclick={onsettings}>Provider settings</button></div>
  <p class="subtle">Answers use passages from your notes with links back to the source. Your configured provider receives the text needed for each request.</p>
  <div class="ai-index-row"><div><strong>Vector index</strong><p class="subtle">Rebuild after changing your notes or embedding model.</p></div><button class="small-button" disabled={busy} onclick={() => confirm('index')}>Build vectors…</button></div>
  {#if error}<p class="dialog-error" role="alert">{error}</p>{/if}
  {#if notice}<p class="subtle" role="status">{notice}</p>{/if}
  <form onsubmit={(event) => { event.preventDefault(); confirm('ask'); }}><label class="field-label">What would you like to explore?<textarea bind:value={question} rows="3" placeholder="What did I learn about this project?" required></textarea></label><button class="primary-button" disabled={busy || !question.trim()}>{busy ? 'Working with your provider…' : 'Ask with sources…'}</button></form>
  {#if answer}
    <div class="ai-answer"><span class="eyebrow">ANSWER · {answer.provider}</span><div class="answer-text">{answer.text}</div></div>
    <section class="citations"><h3>Source passages</h3>{#each answer.citations as citation (citation.id)}<details><summary><span>[{citation.id}] {citation.title}</span><span>Lines {citation.start_line}–{citation.end_line}</span></summary><p>{citation.excerpt}</p><button class="text-button" onclick={() => onselect(citation.path)}>Open {citation.path}</button></details>{:else}<p class="subtle">The provider returned no source citations.</p>{/each}</section>
  {/if}
</section>

{#if confirmation}
  <Dialog label={confirmation === 'index' ? 'Build vectors with your provider' : 'Send a question to your provider'} oncancel={() => (confirmation = null)}>
    <span class="eyebrow">EXTERNAL PROVIDER REQUEST</span><h2>{confirmation === 'index' ? 'Build the vector index?' : 'Ask your configured provider?'}</h2>
    <p>{confirmation === 'index' ? 'This sends the contents of notes in your configured folder scope to your embedding provider. The resulting vectors are a rebuildable local cache.' : 'This sends your question and retrieved note passages to your configured language model provider. Review its settings before continuing.'}</p>
    <details class="reviewed-config" open><summary>Provider and scope being approved</summary><pre>{reviewedConfig}</pre></details>
    {#if confirmation === 'ask'}<p><strong>Question:</strong> {reviewedQuestion}</p>{/if}
    <label class="checkbox-label"><input type="checkbox" bind:checked={acknowledged} />I want to send this text to my configured provider.</label>
    <div class="dialog-actions"><button class="small-button" onclick={() => (confirmation = null)}>Cancel</button><button class="primary-button" disabled={!acknowledged} onclick={perform}>{confirmation === 'index' ? 'Send notes and build' : 'Send question and passages'}</button></div>
  </Dialog>
{/if}
