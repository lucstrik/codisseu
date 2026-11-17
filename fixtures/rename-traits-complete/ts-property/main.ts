import { Weather, OtherWeather } from './weather';
export function propertyReport(weather: Weather): string { return weather.rainfall; }
/* 😀 é */ console.log(new Weather().rainfall, new OtherWeather().rainfall);
// rainfall must stay in this comment
const literal = 'rainfall';
