<?php
require __DIR__ . '/Weather.php';
use ResourceFixture\Weather;
/* 😀 é */ echo (new Weather())->value();
// Weather and Weather.php remain text
$literal = 'Weather';
