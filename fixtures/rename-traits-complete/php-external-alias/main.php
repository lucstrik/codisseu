<?php
use ExternalAliasFixture\Weather;
function externalAliasReport(Weather $weather): int { return $weather->rainfall() + $weather->drizzle(); }
echo externalAliasReport(new Weather());
