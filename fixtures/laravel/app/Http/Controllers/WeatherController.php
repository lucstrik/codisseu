<?php
namespace App\Http\Controllers;
use App\Models\User;
use App\Services\WeatherService;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\Route;
class WeatherController
{
    public function index(Request $request, WeatherService $service): string
    {
        $name = (new User())->displayName();
        return $service->forecast($name);
    }
}
