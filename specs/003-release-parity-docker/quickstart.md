# Quickstart: Validating the Docker Tag Matrix and `paws changelog`

Prerequisites: workspace builds (`cargo build --workspace`), a GitHub token in `$GITHUB_TOKEN`
(or `$GH_TOKEN`) for the changelog scenarios, and — for the SC-004/SC-005/SC-006 end-to-end
scenarios — a local clone of `mbround18/valheim-docker` (already present at
`/home/mbruno/development/rust/valheim-docker` per this feature's spec-review notes) for its real
tag history and existing `CHANGELOG.md` fixture.

## 1) Docker tag rollups (User Story 1)

```bash
cargo test -p paws-docker rollup
```

Manual check against `generate_tags`'s contract (contracts/paws-docker-tag-matrix-contract.md §1):

```bash
paws docker --image myimage --version v3.2.1 --tag-rollup \
  # (fixture git_ref must be a real tag ref, e.g. via a checked-out tag)
```

Expect: `myimage:v3.2.1`, `myimage:3.2`, `myimage:3` in the resolved tag list. Re-run with
`--version v3.2.1-rc.1` (no `--tag-rollup` gate change needed) and confirm zero rollup tags —
SC-002.

## 2) Full tag matrix (User Story 3)

```bash
cargo test -p paws-docker tag_matrix
```

Fixture-driven: one test per trigger shape (branch push, `pull_request`, `schedule`, tag push)
asserting the corresponding `TagKind`s from data-model.md appear, and that omitting every new
flag reproduces `SC-001`'s byte-identical baseline (a fixed fixture snapshot compared against
`generate_tags`'s pre-feature output, run as a regression test — not just "looks right").

## 3) `paws changelog` — local behavior (User Story 2)

```bash
cargo test -p paws-changelog
```

Covers: append-only against a pre-populated fixture (use `valheim-docker`'s actual
`CHANGELOG.md`, copied into the crate's `tests/fixtures/`, per SC-003), first-run file creation,
PR-title rendering against a mocked `HistoryProvider`, raw-commit-subject fallback.

## 4) `paws changelog --commit` — commit-back (FR-013)

Requires a disposable/test repository (do not run `--commit` against a real repo without
`--repository`/`--branch` pointed at a scratch target):

```bash
paws changelog --version v0.0.1-test --previous-ref v0.0.0-test \
  --repository <owner>/<scratch-repo> --branch main --commit
```

Expect: a new commit on `main` with a message containing `[skip ci]`, and the printed entry text
also on stdout. Re-run pointed at a repo whose branch tip was deliberately moved between
`get_content` and `put_content` (e.g. a concurrent push) to confirm the documented loud, no-retry
failure (contracts/paws-changelog-contract.md §2).

## 5) End-to-end against `valheim-docker` (SC-004, SC-005, SC-006)

Dry-run only — do not push against the real `mbround18/valheim-docker` repo:

1. **SC-004**: `paws docker --image mbround18/valheim --version v3.6.1 --tag-rollup` against a
   fixture built from that repo's actual tag ref; confirm a `3` tag is produced.
2. **SC-005**: construct fixtures for each of `docker-release.yml`'s trigger shapes (branch/PR/
   schedule/tag) using that workflow's actual `matrix.image` values (`odin`, `valheim`); confirm
   the tag set for each is a superset of what `ghaction-docker-meta`'s configured `tags:` block
   (`type=schedule`, `type=ref,event=branch`, `type=ref,event=pr`, `type=semver` ×3, `type=sha`)
   would have produced for that shape.
3. **SC-006**: run `paws changelog` (no `--commit`, dry local run only) against a shallow clone
   with `valheim-docker`'s real `CHANGELOG.md` as the pre-existing file and its real
   `v3.6.0..v3.6.1` commit range; confirm the file's pre-existing content (everything above the
   `v3.6.1` boundary this run would add) is byte-for-byte unchanged.

## Definition of done for this quickstart

- `cargo test --workspace` passes with zero failures (spec's Validation Plan, Constitution
  Principle V).
- Every scenario above is backed by an actual `#[test]` in `paws-docker`/`paws-changelog` (tasks.md
  enumerates them 1:1) — this quickstart is a validation guide, not a substitute for the test
  suite itself.
