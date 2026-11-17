<?php
namespace ReceiverFixture;
class Weather { public function rainfall(): int { return 1; } }
class Box { public Weather $item; public function get(): Weather { return new Weather(); } }
function makeWeather(): Weather { return new Weather(); }
