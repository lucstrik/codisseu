import { describe, test, expect } from 'vitest';
import { add } from '../value';
describe('Cálculo 😀', () => {
  test('soma', () => {
    const answer = add(20, 22);
    expect(answer).toBe(42);
  });
  test('falha [literal].*', () => { expect(add(20, 21)).toBe(42); });
  test.skip('ignorado', () => {});
  test.each([['ação 😀', 1], ['outro', 2]])('dataset %s', (_, n) => { expect(n).toBeGreaterThan(0); });
});
