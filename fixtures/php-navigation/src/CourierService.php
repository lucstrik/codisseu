<?php
namespace CodisseuNavigation;

class CourierService extends BaseService implements ParcelGateway {
    public function deliver(): string { return 'courier'; }
}
