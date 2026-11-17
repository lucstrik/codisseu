<?php
namespace AdvancedFixture;
abstract class Port { abstract public function rainfall(string $city = 'Paris', string ...$labels): string; }
