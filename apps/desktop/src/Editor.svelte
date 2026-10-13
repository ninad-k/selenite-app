<script>
  import { onMount } from 'svelte';
  import { Compartment, EditorState } from '@codemirror/state';
  import { EditorView, keymap, lineNumbers, highlightActiveLine, drawSelection } from '@codemirror/view';
  import { markdown } from '@codemirror/lang-markdown';
  import { syntaxHighlighting, defaultHighlightStyle, bracketMatching } from '@codemirror/language';
  import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
  import { searchKeymap, highlightSelectionMatches } from '@codemirror/search';

  let { value = '', onchange, onsave, disabled = false } = $props();
  let element;
  let view = $state(null);
  const editable = new Compartment();

  onMount(() => {
    view = new EditorView({
      parent: element,
      state: EditorState.create({
        doc: value,
        extensions: [
          lineNumbers(), highlightActiveLine(), drawSelection(), history(), markdown(),
          syntaxHighlighting(defaultHighlightStyle), bracketMatching(),
          highlightSelectionMatches(), EditorView.lineWrapping,
          editable.of(EditorView.editable.of(!disabled)),
          keymap.of([
            { key: 'Mod-s', run: () => { onsave?.(); return true; } },
            ...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab,
          ]),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) onchange?.(update.state.doc.toString());
          }),
          EditorView.contentAttributes.of({ 'aria-label': 'Markdown editor', 'data-testid': 'markdown-input', spellcheck: 'true' }),
          EditorView.theme({
            '&': { background: '#fffefb', borderRadius: '8px', border: '1px solid #e1e5da', fontSize: '14px' },
            '&.cm-focused': { outline: '2px solid #94b29b', outlineOffset: '1px' },
            '.cm-content': { fontFamily: '"Cascadia Code", Consolas, monospace', minHeight: '360px', padding: '18px 0', lineHeight: '1.8' },
            '.cm-line': { padding: '0 16px' },
            '.cm-gutters': { backgroundColor: '#f3f5ed', color: '#9ba691', border: '0', borderRadius: '8px 0 0 8px', minWidth: '42px' },
            '.cm-activeLine': { backgroundColor: '#edf3e960' },
            '.cm-activeLineGutter': { backgroundColor: '#e7eddf' },
            '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': { background: '#d9e7d0' },
            '.cm-cursor': { borderLeftColor: '#41655a' },
            '.cm-scroller': { overflow: 'auto' },
          }),
        ],
      }),
    });
    view.focus();
    return () => { view.destroy(); view = null; };
  });

  $effect(() => {
    const enabled = !disabled;
    if (view) view.dispatch({ effects: editable.reconfigure(EditorView.editable.of(enabled)) });
  });
  $effect(() => {
    const text = value;
    if (view && view.state.doc.toString() !== text) view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
  });
</script>

<div bind:this={element} class="markdown-editor" data-testid="markdown-editor"></div>
