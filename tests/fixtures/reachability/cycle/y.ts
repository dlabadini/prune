import { fromX } from "./x";
export function fromY(): number {
  return typeof fromX === "function" ? 1 : 0;
}
