<?php
use AccessorDisabled\Weather;
use AccessorDisabled\OtherWeather;
function reportAccessorDisabled(Weather $source): string { $source->setWeather('rain'); return $source->getWeather() . $source->weather; }
/* 😀 é */ echo reportAccessorDisabled(new Weather()), (new OtherWeather())->getWeather();
// weather and getWeather stay text
$literal='weather';
