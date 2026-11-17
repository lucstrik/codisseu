<?php
use AccessorEnabled\Weather;
use AccessorEnabled\OtherWeather;
function reportAccessorEnabled(Weather $source): string { $source->setWeather('rain'); return $source->getWeather() . $source->weather; }
/* 😀 é */ echo reportAccessorEnabled(new Weather()), (new OtherWeather())->getWeather();
// weather and getWeather stay text
$literal='weather';
