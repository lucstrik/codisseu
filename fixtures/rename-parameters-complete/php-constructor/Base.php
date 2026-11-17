<?php
namespace ParamConstructors;
class Base { public string $value; public function __construct(string $city) { $this->value = $city; } }
class Child extends Base {}
