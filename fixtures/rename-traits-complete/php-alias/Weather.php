<?php
namespace AliasFixture;
class Weather { use Rain { Rain::rainfall as drizzle; } }
