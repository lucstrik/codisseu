<?php
namespace PropertyFixture;
class Weather {
    public string $rainfall = 'wet';
    public function read(): string { return $this->rainfall; }
}
class OtherWeather { public string $rainfall = 'other'; }
