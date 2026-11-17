<?php

declare(strict_types=1);

use PHPUnit\Framework\Attributes\DataProvider;
use PHPUnit\Framework\TestCase;

final class ExampleTest extends TestCase
{
    public function testPasses(): void { self::assertSame(42, 40 + 2); }
    public function testFails(): void { self::assertSame(42, 41, 'falha ação 😀'); }
    public function testErrors(): void { throw new RuntimeException('erro controlado'); }
    public function testSkips(): void { self::markTestSkipped('ignorado de propósito'); }
    #[DataProvider('numbers')]
    public function testDataset(int $number): void { self::assertGreaterThan(0, $number); }
    public static function numbers(): array { return ['ação 😀' => [1], 'second' => [2]]; }
}
