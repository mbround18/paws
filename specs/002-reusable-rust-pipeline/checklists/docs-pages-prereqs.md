# Docs Publish Prerequisites

- [x] Repository has GitHub Pages configured for Actions deployment (`github-pages` environment).
- [x] Workflow has `pages: write` and `id-token: write` only in docs deployment job.
- [x] Rust docs output path is configurable via `docs_path`.
- [x] Docs deployment is opt-in through `publish_docs`.
- [x] Docs path validation emits actionable failure guidance before upload/deploy.
