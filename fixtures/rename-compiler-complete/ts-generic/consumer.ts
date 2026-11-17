import { Weather } from './weather';
export function consumer(source: Weather<number>): number { return source.rainfall(3); }
