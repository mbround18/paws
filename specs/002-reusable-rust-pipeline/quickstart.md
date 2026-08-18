# Quickstart Validation Guide: Reusable Rust Pipeline Architecture

This guide validates contract behavior for baseline Rust CI, optional crates publish, and optional docs publication.

## Prerequisites

- Node + pnpm installed
- Workspace dependencies installed

## Setup

```bash
pnpm install
```

## Scenario A: Governance baseline for reusable workflow + Dagger contracts

```bash
pnpm --filter @gh-reusable/dagger-pipelines run test -- src/workflow-governance.test.ts
pnpm --filter @gh-reusable/dagger-pipelines run test -- src/dagger-module-integration.test.ts
```

Expected outcome:

- Workflow inputs/secrets and Dagger call names are contract-valid.

## Scenario B: Rust publish/release compatibility surfaces

```bash
pnpm --filter @gh-reusable/dagger-pipelines run test -- src/publish-workflow.test.ts src/release-workflows.test.ts
```

Expected outcome:

- Existing publish contract remains compatible.
- Rust publish path still maps to expected Dagger contract entrypoints.

## Scenario C: Optional docs publication contract and failure messaging

```bash
pnpm --filter @gh-reusable/dagger-pipelines run test -- src/rust-workflow.test.ts src/release-workflows.test.ts
```

Expected outcome:

- Docs mode remains opt-in via `publish_docs`.
- Reusable docs workflow wiring (`rust-docs-publish.yaml`) is present.
- Actionable docs-path failure guidance is enforced in tests.

## Scenario D: Runtime defaults + permission posture

```bash
pnpm --filter @gh-reusable/dagger-pipelines run test -- src/standards-defaults.test.ts src/security-workflows.test.ts src/audit-workflow.test.ts
```

Expected outcome:

- Rust defaults align with `defaults.json`.
- Reusable workflows continue declaring explicit permissions and security behavior.

## Scenario E: Full workspace confidence

```bash
pnpm run typecheck
pnpm run test
```

Expected outcome:

- No cross-package regressions.

## Focused compliance aliases

```bash
pnpm --filter @gh-reusable/dagger-pipelines run test:compliance:us1
pnpm --filter @gh-reusable/dagger-pipelines run test:compliance:us2
pnpm --filter @gh-reusable/dagger-pipelines run test:compliance:us3
pnpm --filter @gh-reusable/dagger-pipelines run test:compliance
```

## Traceability

- Plan: `specs/002-reusable-rust-pipeline/plan.md`
- Research decisions: `specs/002-reusable-rust-pipeline/research.md`
- Data entities/modes: `specs/002-reusable-rust-pipeline/data-model.md`
- Contract rules: `specs/002-reusable-rust-pipeline/contracts/reusable-rust-pipeline-contract.md`
