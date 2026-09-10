# ansible-fixture

The target for `paws ci --toolchain ansible`: a `uv`-managed Ansible control
repo, in miniature. It has the four things the pipeline keys off — a
`pyproject.toml` pinning `ansible-core` and `ansible-lint`, a
`requirements.yml` naming a collection that is *not* bundled with
`ansible-core`, a `playbooks/` directory, and a role under `roles/` — so a
run exercises the galaxy install, the lint, and one `--syntax-check` per
playbook.

The `ansible.posix.synchronize` task is `when: false` and never executes;
it exists so the playbook genuinely fails to resolve without the galaxy
step, which is what makes that step's ordering testable end to end.
