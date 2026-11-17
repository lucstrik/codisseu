import { Forecast } from './forecast';
import { Weather } from './weather';
import { WetWeather } from './wetWeather';
function report(forecast: Forecast): string { return forecast.rainfall(); }
// rainfall remains in comments and strings.
console.log(report(new Weather()), report(new WetWeather()));
