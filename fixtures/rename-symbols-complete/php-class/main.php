<?php
use ClassFixture\CloudSource;
function classReport(CloudSource $source): string { return $source->value(); }
/* 😀 é */ echo classReport(new CloudSource());
// CloudSource stays here
$literal = 'CloudSource';
