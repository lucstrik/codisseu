<?php
namespace VarianceFixture;
class Animal {}
class Dog extends Animal {}
abstract class Port { abstract public function rainfall(Dog $animal): Animal; }
