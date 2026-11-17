<?php
namespace ContractFixture;
class OtherWeather {
    public function rainfall(string $city): string { return $city . ': unrelated rainfall'; }
}
