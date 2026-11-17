describe('Cálculo 😀', () => {
  test('soma', () => {
    const answer = 20 + 22;
    expect(answer).toBe(42);
  });
  test('falha [literal].*', () => { expect(41).toBe(42); });
  test.skip('ignorado', () => {});
  test.each([['ação 😀', 1], ['outro', 2]])('dataset %s', (_, n) => { expect(n).toBeGreaterThan(0); });
});
