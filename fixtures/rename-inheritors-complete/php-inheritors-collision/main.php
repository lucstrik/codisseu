<?php
namespace InheritorsCollision;
require __DIR__ . '/Weather.php';
require __DIR__ . '/WetWeather.php';
require __DIR__ . '/WeatherChild.php';
require __DIR__ . '/ChildWeatherChild.php';
new Weather();
new WetWeather();
new WeatherChild();
new ChildWeatherChild();
// Weather WetWeather WeatherChild remain in comments
