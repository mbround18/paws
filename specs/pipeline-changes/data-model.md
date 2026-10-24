# Data Model: Mod Upload Pipeline

## Entities

### `UploadConfig`

```rust
struct UploadConfig {
    provider: Provider,
    environment: Option<String>,
    validate_only: bool,
    dry_run: bool,
    package_path: PathBuf,
    artifact_name: Option<String>,
    team: Option<String>,
    access_key: Option<String>,
    username: Option<String>,
    password: Option<String>,
    publish_on_success: bool,
    language_matrix: Vec<String>,
    mod_root: PathBuf,
    build_target: Option<String>,
    test_target: Option<String>,
    package_target: Option<String>,
}
```

**Purpose**: Encapsulates the caller-controlled upload and mod-build configuration. It is the top-level contract used by workflow inputs and Dagger orchestration.

### `CredentialSet`

```rust
enum CredentialSet {
    AccessKey(String),
    UserPasswordTeam {
        username: String,
        password: String,
        team: String,
    },
}
```

**Validation rules**:
- `AccessKey` is valid for provider(s) that support direct key auth.
- `UserPasswordTeam` requires non-empty `username`, `password`, and `team`.
- A credential set may not be silently combined if both patterns are present; the workflow must resolve one deterministic source.

### `ValidationResult`

```rust
struct ValidationResult {
    ok: bool,
    provider: Provider,
    errors: Vec<String>,
    warnings: Vec<String>,
    package_sha256: Option<String>,
}
```

**Purpose**: Represents the pre-upload validation stage outcome for an archive or package bundle.

### `BuildResult`

```rust
struct BuildResult {
    ok: bool,
    language_matrix: Vec<String>,
    output_dir: Option<PathBuf>,
    errors: Vec<String>,
}
```

**Purpose**: Records the build stage for a mixed-language project.

### `TestResult`

```rust
struct TestResult {
    ok: bool,
    language_matrix: Vec<String>,
    failures: Vec<String>,
}
```

**Purpose**: Captures the test stage and blocks packaging/upload when tests fail.

### `PackageResult`

```rust
struct PackageResult {
    ok: bool,
    path: Option<PathBuf>,
    manifest: Option<String>,
    errors: Vec<String>,
}
```

**Purpose**: Represents the final built package artifact that will be validated and optionally uploaded.

### `PipelineResult`

```rust
struct PipelineResult {
    build: BuildResult,
    test: TestResult,
    package: PackageResult,
    validation: ValidationResult,
    upload: Option<UploadResult>,
    status: PipelineStatus,
}
```

**Purpose**: Final outcome for the full mod workflow.

### `UploadResult`

```rust
struct UploadResult {
    success: bool,
    provider: Provider,
    upload_url: Option<String>,
    publish_id: Option<String>,
    error: Option<String>,
}
```

**Purpose**: Captures the external API result after validation has succeeded.

## Relationships

- `UploadConfig` resolves to one `CredentialSet`.
- `BuildResult`, `TestResult`, `PackageResult`, and `ValidationResult` together define a `PipelineResult`.
- `UploadResult` is optional because `validate_only` or `dry_run` may stop before remote publication.
- A `Valheim Mod Project` is the composition of build/test/package stages and the final package artifact.

## State transitions

- `initialized` → `build_passed` → `test_passed` → `package_created` → `validated` → `uploaded`
- Any failing stage transitions to `failed` and stops the workflow.
- `validate_only` or `dry_run` ends in `validated` or `dry-run-complete` without a remote upload.
