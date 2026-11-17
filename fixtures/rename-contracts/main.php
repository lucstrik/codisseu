<?php
require_once __DIR__ . '/vendor/autoload.php';
use ContractFixture\Weather;
use ContractFixture\WeatherPort;
$weather = new Weather();
echo '🌧 é'; echo $weather->rainfall('Porto');
function reportWeather(WeatherPort $source): string { return $source->rainfall('Recife'); }
echo reportWeather($weather);
// rainfall remains a comment; 'rainfall' remains a string.
