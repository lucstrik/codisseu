export interface Developer {
  id: number;
  name: string;
}

const developer: Developer = { id: 1, name: "Storm" };
export const greeting = (user: Developer): string => `Olá, ${user.name}!`;
console.log(greeting(developer));
