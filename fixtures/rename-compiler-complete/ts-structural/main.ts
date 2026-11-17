import { Port, source } from './port';
function show(port: Port): string { return port.rainfall; }
/* 😀 é */ console.log(show(source), source.rainfall);
