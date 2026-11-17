<?php
namespace App\Services;
final class WeatherService
{
    public function forecast(string $city): string { return $city . ':sun'; }
}
