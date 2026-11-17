<?php
namespace AccessorDisabled;
class Weather {
    public string $weather = 'wet';
    public function getWeather(): string { return $this->weather; }
    public function setWeather(string $value): void { $this->weather = $value; }
}
class OtherWeather { public function getWeather(): string { return 'other'; } }
