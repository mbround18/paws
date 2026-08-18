# GH Reusable CI Standards Preset

This preset applies `gh-reusable` standards to Spec Kit artifacts so pipeline changes stay contract-safe and auditable.

## What it enforces

- Dagger-first workflow behavior and call contract clarity.
- Reusable workflow input/secret/output compatibility discipline.
- Runtime baseline governance through `defaults.json`.
- Security and permissions impact accounting.
- Explicit validation + rollout/rollback planning.

## Install (local dev)

```bash
specify preset add --dev ./presets/gh-reusable
```

## Resolve examples

```bash
specify preset resolve spec-template
specify preset resolve plan-template
specify preset resolve tasks-template
```
