export class Weather { rainfall(): number { return 1; } }
export class Box { item: Weather = new Weather(); get(): Weather { return new Weather(); } }
export function makeWeather(): Weather { return new Weather(); }
