import test from 'node:test';
import assert from 'node:assert/strict';
import { cellText, parseCell, calendarRows, monthDays } from './cells.js';

test('property editor preserves typed values, zero, and optional empty cells', () => {
  assert.deepEqual(parseCell({ name: 'effort', type: 'number' }, '0'), { type: 'number', value: 0 });
  assert.deepEqual(parseCell({ name: 'effort', type: 'number' }, undefined), { type: 'empty' });
  assert.deepEqual(parseCell({ name: 'done', type: 'checkbox' }, false), { type: 'checkbox', value: false });
  assert.deepEqual(parseCell({ name: 'tags', type: 'multi_select' }, ['home', 'work, meetings']), { type: 'multi_select', value: ['home', 'work, meetings'] });
  assert.deepEqual(parseCell({ name: 'projects', type: 'relation' }, '[[Project A, B]]\nProject C'), { type: 'relation', value: ['[[Project A, B]]', 'Project C'] });
  assert.deepEqual(parseCell({ name: 'projects', type: 'relation' }, ''), { type: 'empty' });
  assert.equal(cellText({ type: 'number', value: 0 }), '0');
  assert.equal(cellText({ type: 'empty' }), '');
  assert.throws(() => parseCell({ name: 'effort', type: 'number' }, 'Infinity'), /finite number/);
});

test('calendar groups dates without timezone changes and retains undated rows', () => {
  const rows = [
    { path: 'a.md', cells: { date: { type: 'date', value: '2026-10-14' } } },
    { path: 'b.md', cells: {} },
    { path: 'c.md', cells: { date: { type: 'date', value: '2026-10-13' } } },
  ];
  assert.deepEqual(calendarRows(rows, 'date').map(([date, group]) => [date, group.map((row) => row.path)]), [
    ['2026-10-13', ['c.md']], ['2026-10-14', ['a.md']], ['', ['b.md']],
  ]);
  const days = monthDays('2024-02');
  assert.equal(days.filter(Boolean).length, 29);
  assert.equal(days[0], null);
  assert.equal(days[3], '2024-02-01');
  assert.equal(days.at(-1), '2024-02-29');
  assert.deepEqual(monthDays('2026-13'), []);
});
