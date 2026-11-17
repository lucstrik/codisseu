<?php
use ResourceFixture\Weather as Sky;
function resourceReport(Sky $source): string { return $source->value(); }
