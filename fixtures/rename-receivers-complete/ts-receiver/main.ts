import { Box, makeWeather } from './weather';
const source = makeWeather();
/* 😀 é */ console.log(new Box().item.rainfall(), new Box().get().rainfall(), source.rainfall());
