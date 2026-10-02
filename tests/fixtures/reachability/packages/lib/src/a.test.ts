import { testOnlyA } from "./a";
if (testOnlyA() !== "tested") throw new Error("broken");
