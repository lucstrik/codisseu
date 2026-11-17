<?php
namespace ContractFixture;
class Weather implements WeatherPort {
    public function rainfall(string $city): string { return $city . ': rainfall'; }
}
