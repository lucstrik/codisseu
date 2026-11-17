<?php
use ClassFixture\CloudSource as Sky;
function aliasedClassReport(Sky $source): string { return $source->value(); }
