export function cellText(cell) {
  if (!cell || cell.type === 'empty') return '';
  if (Array.isArray(cell.value)) return cell.value.join(', ');
  return String(cell.value);
}

export function parseCell(field, value) {
  if (field.type === 'checkbox') return { type: 'checkbox', value: Boolean(value) };
  if (field.type === 'multi_select' || field.type === 'relation') {
    const values = Array.isArray(value) ? value : String(value ?? '').split(/\r?\n/);
    const entries = values.map((part) => part.trim()).filter(Boolean);
    return entries.length ? { type: field.type, value: entries } : { type: 'empty' };
  }
  const text = String(value ?? '').trim();
  if (!text) return { type: 'empty' };
  if (field.type === 'number') {
    const number = Number(text);
    if (!Number.isFinite(number)) throw new Error(`${field.name} must be a finite number.`);
    return { type: 'number', value: number };
  }
  return { type: field.type, value: text };
}

export function calendarRows(rows, field) {
  const groups = new Map();
  for (const row of rows) {
    const value = row.cells[field];
    const key = value?.type === 'date' ? value.value : '';
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(row);
  }
  return [...groups].sort(([left], [right]) => left ? (right ? left.localeCompare(right) : -1) : 1);
}

export function monthDays(month) {
  if (!/^\d{4}-\d{2}$/.test(month)) return [];
  const [year, number] = month.split('-').map(Number);
  if (number < 1 || number > 12) return [];
  const first = new Date(Date.UTC(year, number - 1, 1));
  const offset = (first.getUTCDay() + 6) % 7;
  const count = new Date(Date.UTC(year, number, 0)).getUTCDate();
  return [
    ...Array(offset).fill(null),
    ...Array.from({ length: count }, (_, index) => `${month}-${String(index + 1).padStart(2, '0')}`),
  ];
}
