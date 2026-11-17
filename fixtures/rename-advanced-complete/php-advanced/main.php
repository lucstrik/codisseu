<?php
use AdvancedFixture\Port;
use AdvancedFixture\Weather;
function advancedReport(Port $source): string { return $source->rainfall(); }
/* 😀 é */ echo advancedReport(new Weather()), (new Weather())->rainfall('London','wet');
// rainfall stays text
