<?php

test('passes ação 😀', function () { expect(40 + 2)->toBe(42); });
it('fails', function () { expect(41)->toBe(42); });
test('errors', function () { throw new RuntimeException('erro Pest'); });
test('skipped', function () { expect(true)->toBeTrue(); })->skip('intencional');
test('dataset', function (int $value) { expect($value)->toBeGreaterThan(0); })->with(['ação 😀' => 1, 'second' => 2]);
