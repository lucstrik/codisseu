import { CloudSource } from './weather';
export function classReport(source: CloudSource): string { return source.value(); }
/* 😀 é */ console.log(classReport(new CloudSource()));
// CloudSource stays here
const literal = 'CloudSource';
