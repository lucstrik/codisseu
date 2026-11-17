import { Port } from './port';
export class Weather implements Port { rainfall<U extends string = string>(city: U, label?: string, ...extra: string[]): U { return city; } }
