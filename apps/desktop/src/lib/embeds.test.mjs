import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeEmbedUrl } from './embeds.js';

test('embeds require an explicit HTTPS origin and reject inline or credential-bearing sources', () => {
  for (const value of ['javascript:alert(1)', 'data:text/html,hi', 'file:///note.md', 'http://example.com/video', 'https://user:password@example.com/embed']) assert.throws(() => normalizeEmbedUrl(value));
  assert.deepEqual(normalizeEmbedUrl('https://video.example/course?id=1'), { url: 'https://video.example/course?id=1', origin: 'https://video.example' });
});

test('ordinary Google Drive file links become preview URLs without trusting lookalike domains', () => {
  assert.equal(normalizeEmbedUrl('https://drive.google.com/file/d/Ab_cd-123/view?resourcekey=abc').url, 'https://drive.google.com/file/d/Ab_cd-123/preview?resourcekey=abc');
  assert.equal(normalizeEmbedUrl('https://drive.google.com.example/file/d/id/view').url, 'https://drive.google.com.example/file/d/id/view');
});
