---
applyTo: "**/*.{yml,yaml,rs,md}"
---

# Release and CI instructions

- Treat release, publish, and registry actions as opt-in and safe-by-default.
- Prefer environment-based auth and validation gates before upload or publication.
- Ensure workflows fail clearly with structured reasons instead of vague runtime errors.
- Keep release docs and examples aligned with the actual command surface.
- Do not add flaky or environment-specific behavior to CI steps.
- Validate with the relevant repo quality gate before completion.
