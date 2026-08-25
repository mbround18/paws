<?php

declare(strict_types=1);

namespace Example\Tests;

use Example\Calculator;
use PHPUnit\Framework\TestCase;

final class CalculatorTest extends TestCase
{
    public function testAdd(): void
    {
        self::assertSame(5, (new Calculator())->add(2, 3));
    }

    public function testMultiply(): void
    {
        self::assertSame(6, (new Calculator())->multiply(2, 3));
    }
}
