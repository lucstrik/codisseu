import { Weather } from './src/weather';
import type { WeatherPort } from './src/weatherPort';
const weather = new Weather();
console.log('🌧 é'); console.log(weather.rainfall('Porto'));
function reportWeather(source: WeatherPort): string { return source.rainfall('Recife'); }
console.log(reportWeather(weather));
// rainfall remains a comment; 'rainfall' remains a string.
