// Verifies the Next.js build actually produced a real production build
// output, not just that `next build` was invoked.
import assert from "node:assert/strict";
import { existsSync } from "node:fs";

assert.ok(existsSync(".next/BUILD_ID"), "expected .next/BUILD_ID to exist after `npm run build`");
console.log("ok: .next/BUILD_ID exists");
