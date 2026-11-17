<?php
namespace AdvancedFixture;
class Weather extends Port { public function rainfall(string $city = 'Paris', string ...$labels): string { return $city; } }
