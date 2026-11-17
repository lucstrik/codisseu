<?php
use PropertyFixture\Weather;
use PropertyFixture\OtherWeather;
function propertyReport(Weather $weather): string { return $weather->rainfall; }
/* 😀 é */ echo (new Weather())->rainfall, (new OtherWeather())->rainfall;
// rainfall must stay in this comment
$literal = 'rainfall';
