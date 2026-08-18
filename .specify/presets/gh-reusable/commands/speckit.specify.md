---
description: Create a pipeline-change specification aligned to gh-reusable CI standards.
---

Create or update a feature spec for this pipeline/workflow change using the `spec-template`.

Hard requirements:

1. Treat reusable workflow inputs/secrets/outputs and Dagger `@func()` signatures as public contracts.
2. Explicitly identify whether the change is backward-compatible.
3. Call out any runtime baseline changes (`defaults.json`) and container implications.
4. Document explicit permission changes and security scan behavior changes.
5. Include a concrete validation plan and rollback strategy.

Output path should be under `specs/pipeline-changes/`.
