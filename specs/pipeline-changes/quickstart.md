# Quickstart: Validate the Mod Upload Pipeline

## Prerequisites

- GitHub Actions access to the target repository
- GitHub Environment configured for the publish target
- Valid registry credentials in the environment, either as `ACCESS_KEY` or `USERNAME`/`PASSWORD` + `TEAM`
- A mod package built from a repo that includes the required language matrix and final package manifest

## Validation scenarios

### 1. Validate before upload

```bash
# Example: validation-only mode for a package bundle
paws mod validate --provider thunderstore --package ./dist/mod.zip --dry-run
```

Expected result:
- validation runs and produces a structured pass/fail result
- no remote upload call is made
- exit code is non-zero on validation failure

### 2. Publish to Hexium using environment access key

```bash
export ACCESS_KEY="<hexium-key>"
paws mod upload --provider hexium --package ./dist/mod.zip --environment production --validate
```

Expected result:
- environment secret is resolved from the GitHub Environment, not repo-global config
- validation passes, then upload request is sent
- job returns success with remote publish metadata if accepted

### 3. Publish to Thunderstore using username/password/team

```bash
export USERNAME="<team-user>"
export PASSWORD="<team-password>"
export TEAM="<team-name>"
paws mod upload --provider thunderstore --package ./dist/mod.zip --environment production --validate
```

Expected result:
- credentials are resolved correctly
- manifest validation happens before upload
- remote call is only attempted when validation succeeds

### 4. End-to-end valheim-mods release path

```bash
paws mod pipeline \
  --repo . \
  --language-matrix rust,csharp,other \
  --provider thunderstore \
  --environment production \
  --publish-on-success
```

Expected result:
- build phase runs across the configured language matrix
- test phase blocks the pipeline if any target fails
- final artifact is packaged and validated
- upload occurs only after validation passes

## Expected outcomes

- Failing validation blocks upload.
- Missing required credential sets fail fast with a structured reason.
- Existing workflows without upload configuration remain unaffected.
- Build/test/package stages are independently observable and replayable.
