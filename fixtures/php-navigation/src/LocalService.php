<?php
namespace CodisseuNavigation;

class LocalService implements ParcelGateway {
    public function deliver(): string { return 'local'; }
}
