<?php
use ReceiverFixture\Box;
use function ReceiverFixture\makeWeather;
function show(Box $box): int { return $box->item->rainfall(); }
/* 😀 é */ echo (new Box())->get()->rainfall(), makeWeather()->rainfall();
