import { CloudSource as Sky } from './weather';
export function aliasedClassReport(source: Sky): string { return source.value(); }
