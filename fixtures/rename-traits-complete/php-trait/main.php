<?php
use TraitFixture\Weather;
function traitfixtureReport(Weather $weather): int { return $weather->rainfall() + $weather->drizzle(); }
echo traitfixtureReport(new Weather());
