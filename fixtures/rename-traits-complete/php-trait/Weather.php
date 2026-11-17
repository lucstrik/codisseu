<?php
namespace TraitFixture;
class Weather { use Rain { Rain::rainfall as drizzle; } }
