<?php
require_once __DIR__ . '/vendor/autoload.php';
use ContractFixture\OtherWeather;
$weather = new OtherWeather();
echo $weather->rainfall('Bairro');
