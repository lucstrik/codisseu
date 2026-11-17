<?php
namespace ExternalAliasFixture;
class Weather { use Rain { Rain::rainfall as drizzle; } }
