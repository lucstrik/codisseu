<?php
use AliasFixture\Weather;
function aliasfixtureReport(Weather $weather): int { return $weather->rainfall() + $weather->drizzle(); }
echo aliasfixtureReport(new Weather());
