import test from 'node:test';
import assert from 'node:assert/strict';
import { dirtyConfigurations, serviceDefaults, updateConfiguration } from './services.js';

test('service drafts preserve comparison source and undo restores a clean state', () => {
  const saved = { expectedSource: 'enabled = false\n', source: 'enabled = false\n', original: 'enabled = false\n', dirty: false };
  const changed = updateConfiguration(saved, 'enabled = true\n');
  assert.equal(changed.expectedSource, saved.expectedSource);
  assert.equal(changed.dirty, true);
  assert.deepEqual(dirtyConfigurations({ ai: changed, sync: saved }).map(([service]) => service), ['ai']);
  assert.equal(updateConfiguration(changed, saved.original).dirty, false);
});

test('suggested configurations disable remote services and contain no credential literals', () => {
  for (const service of ['ai', 'sync', 'plugins']) assert.match(serviceDefaults[service], /^enabled = false/m);
  assert.match(serviceDefaults.pro, /^\[trusted_keys\]$/m);
  assert.doesNotMatch(serviceDefaults.pro, /^\w+\s*=\s*".+"$/m);
  assert.match(serviceDefaults.ai, /^api_key_env = "SELENITE_LLM_API_KEY"$/m);
  assert.doesNotMatch(serviceDefaults.ai, /^api_key\s*=/m);
});
