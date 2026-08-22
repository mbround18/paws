import assert from "node:assert/strict";
import { createApp } from "./server.js";

const app = createApp();

await new Promise((resolve) => app.listen(0, resolve));
const { port } = app.address();

const res = await fetch(`http://127.0.0.1:${port}/health`);
assert.equal(res.status, 200);
assert.deepEqual(await res.json(), { status: "ok" });

app.close();
console.log("ok");
