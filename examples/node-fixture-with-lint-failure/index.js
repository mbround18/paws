export function add(a, b) {
  return a + b;
}

export function debugAdd(a, b) {
  // Deliberate lint violation: console.log is banned by lint.js's "no-console-log"
  // rule (see lint.js / examples/README.md for why this fixture exists).
  console.log("adding", a, b);
  return a + b;
}
