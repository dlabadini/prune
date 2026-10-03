import fs from "node:fs";
import path from "node:path";
import { used, withDefaults } from "./lib/util.js";

const read = (...parts) => fs.readFileSync(path.join(process.cwd(), ...parts), "utf8");

function wants(req) {
  return req.accepts(["text/html", "text/markdown"]);
}

console.log(read("views", "index.html").length, wants, used(1), withDefaults({}));
