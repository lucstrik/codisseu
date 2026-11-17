<?php
use CodisseuNavigation\ParcelGateway;
use CodisseuNavigation\ParcelFactory;
use CodisseuNavigation\CourierService;
use function CodisseuNavigation\quote_label;

function inspect(ParcelGateway $gateway): string {
    return $gateway->deliver();
}

// ação 😀: navigation after a non-BMP character.
echo '😀'; $gateway = ParcelFactory::create();
$gateway->deliver();
(new CourierService())->inherited();
quote_label();
// Never resolve this comment as a call: missing_symbol().
missing_symbol();
