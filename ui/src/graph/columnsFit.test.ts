import { describe, expect, it } from 'vitest';
import { allocateColumns, COLUMN_MIN, DEFAULT_COLUMN_PREFS } from './columns';

describe('allocateColumns with a stored (hand-set) Message width (S.1)', () => {
  const p = { ...DEFAULT_COLUMN_PREFS, graph: 64, message: 400 };
  const fixed = p.labels + 64 + p.author + p.date + p.sha;
  it('a wide window leaves empty space instead of growing Message', () => {
    const w = allocateColumns(p, fixed + 400 + 300);
    expect(w.message).toBe(400);
    expect(w.total).toBe(fixed + 400);
  });
  it('a narrower window scrolls instead of squeezing: Message, Author and Date keep their widths', () => {
    for (const available of [fixed + 250, fixed + COLUMN_MIN.message - 30, 300]) {
      const w = allocateColumns(p, available);
      expect(w, `available ${available}`).toMatchObject({ message: 400, author: p.author, date: p.date, total: fixed + 400 });
      expect(w.total).toBeGreaterThan(available);
    }
  });
  it('wider than the window (20 lanes of graph), it is still honoured and the table scrolls', () => {
    const wide = { ...p, graph: 20 * 16 + 16, message: 900 };
    const w = allocateColumns(wide, 800);
    expect(w).toMatchObject({ graph: 336, message: 900, author: p.author, date: p.date });
    expect(w.total).toBe(p.labels + 336 + 900 + p.author + p.date + p.sha);
  });
  it('null still fills the table, squeezing Author and Date once Message is at its minimum', () => {
    expect(allocateColumns({ ...p, message: null }, fixed + 700).message).toBe(700);
    const tight = allocateColumns({ ...p, message: null }, fixed + COLUMN_MIN.message - 30);
    expect(tight.message).toBe(COLUMN_MIN.message);
    expect(tight.author + tight.date).toBe(p.author + p.date - 30);
    expect(tight.total).toBe(fixed + COLUMN_MIN.message - 30);
  });
});
