import { Port } from './port';
import { Weather } from './weather';
export function advancedReport(source: Port): string { return source.rainfall('Paris'); }
/* 😀 é */ console.log(advancedReport(new Weather()),new Weather().rainfall('London','wet'));
// rainfall stays text
