# Mod Upload Contract

## Reusable Workflow Inputs

```yaml
inputs:
  provider:
    required: true
    type: string
  environment:
    required: false
    type: string
  mod_root:
    required: true
    type: string
  language_matrix:
    required: false
    type: string
  build_target:
    required: false
    type: string
  test_target:
    required: false
    type: string
  package_target:
    required: false
    type: string
  package_path:
    required: true
    type: string
  validate_only:
    required: false
    type: boolean
  dry_run:
    required: false
    type: boolean
  access_key:
    required: false
    type: string
  username:
    required: false
    type: string
  password:
    required: false
    type: string
  team:
    required: false
    type: string
```

## Reusable Workflow Secrets

- `ACCESS_KEY`
- `USERNAME`
- `PASSWORD`
- `TEAM`

Resolution order:
1. GitHub Environment value
2. Workflow env value
3. Repository secret fallback
4. Failure if required values are absent

## Public Output Contract

```yaml
outputs:
  status:
    value: ${{ steps.upload.outputs.status }}
  provider:
    value: ${{ steps.upload.outputs.provider }}
  validation_result:
    value: ${{ steps.upload.outputs.validation_result }}
  artifact:
    value: ${{ steps.upload.outputs.artifact }}
  upload_url:
    value: ${{ steps.upload.outputs.upload_url }}
  publish_id:
    value: ${{ steps.upload.outputs.publish_id }}
```

## Dagger Public Function Contract

```rust
#[func]
pub fn build_mod_artifact(
    repo_root: Path,
    language_matrix: Vec<String>,
    build_target: Option<String>,
) -> BuildResult;

#[func]
pub fn test_mod_artifact(
    repo_root: Path,
    language_matrix: Vec<String>,
    test_target: Option<String>,
) -> TestResult;

#[func]
pub fn package_mod_artifact(
    repo_root: Path,
    package_target: Option<String>,
    manifest: ModManifest,
) -> PackageResult;

#[func]
pub fn validate_mod_package(
    provider: Provider,
    package_path: Path,
    metadata: ModMetadata,
) -> ValidationResult;

#[func]
pub fn publish_mod_package(
    provider: Provider,
    package_path: Path,
    credentials: UploadCredentials,
    team: Option<String>,
    dry_run: bool,
) -> UploadResult;
```

## Error semantics

- Missing required secrets or provider configuration fails before any network upload.
- Validation failure produces a structured, machine-readable reason.
- `validate_only` or `dry_run` returns a status object without publishing.
- The workflow must be safe by default and must not publish without an explicit opt-in.
