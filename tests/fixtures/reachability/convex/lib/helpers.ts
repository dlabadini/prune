import { makeFunctionReference } from "convex/server";

export function helper() {
  return makeFunctionReference<"query">("messages:viaString");
}

export function unusedHelper() {
  return 1;
}
