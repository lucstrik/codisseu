import { Weather } from './weather';
const source: Weather<string> = new Weather<string>();
/* 😀 é */ console.log(source.rainfall('Paris'));
