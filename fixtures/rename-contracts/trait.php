<?php
require_once __DIR__ . '/vendor/autoload.php';
use ContractFixture\TraitWeather;
$weather = new TraitWeather();
echo $weather->rainfall('Olinda');
// rainfall remains a comment.
