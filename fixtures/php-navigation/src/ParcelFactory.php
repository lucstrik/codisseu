<?php
namespace CodisseuNavigation;

class ParcelFactory {
    public static function create(): ParcelGateway { return new CourierService(); }
}
