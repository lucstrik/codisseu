import { Forecast } from './forecast';
export class WetWeather implements Forecast {
    rainfall(): string { return 'wet rainfall'; }
}
