import assert from "node:assert/strict";
import { subtract } from "./index.js";

assert.equal(subtract(5, 3), 2);
console.log("ok");
