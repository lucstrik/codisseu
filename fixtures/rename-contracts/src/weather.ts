import type { WeatherPort } from './weatherPort';
export class Weather implements WeatherPort {
    rainfall(city: string): string { return city + ': rainfall'; }
}
