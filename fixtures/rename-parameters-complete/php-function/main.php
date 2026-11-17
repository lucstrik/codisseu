<?php
use function ParamFunctions\greet as hello;
use function ParamFunctions\other;
echo hello(city: 'Paris');
echo other(city: 'Lima');
