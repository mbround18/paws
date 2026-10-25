---
applyTo: "**/*.rs"
---

# Rust coding instructions

- Keep each crate focused and small. Prefer the existing crate boundaries over cross-cutting abstractions.
- Use idiomatic Rust and explicit types. Avoid unnecessary boxing, trait proliferation, or hidden magic.
- Follow the repo’s strict warning policy: no warnings, no dead code, no unused imports.
- Prefer `cargo check -p <crate>` during the edit/build loop; reserve full workspace validation for the final pass.
- Keep public APIs stable unless intentionally breaking and documented.
- For external integrations, prefer validated config objects and structured error types rather than ad hoc strings.
- Never leak credentials or tokens in error messages or command output.
