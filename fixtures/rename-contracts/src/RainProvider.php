<?php
namespace ContractFixture;
trait RainProvider {
    public function rainfall(string $city): string { return $city . ': rainfall'; }
}
