<?php
use App\Http\Controllers\WeatherController;
use Illuminate\Support\Facades\Route;
Route::get('/weather', [WeatherController::class, 'index'])->name('weather.index');
Route::name('admin.')->middleware('auth')->group(function () {
    Route::get('/weather', [WeatherController::class, 'index'])->name('weather.index');
});
