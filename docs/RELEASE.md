# Release process

All four crates (`remarkable-metadata`, `remarkable-tablet`, `remarkable-rm`, `remarkable-cli`) share one version (`[workspace.package].version` in `Cargo.toml`) and are always released together. Releases are done by [release-plz](https://release-plz.dev) through two manually triggered GitHub Actions workflows. Version bumps come from conventional commit messages. While on 0.x: `fix:`/`feat:` → patch (0.1.0 → 0.1.1, verified), `feat!:`/`BREAKING CHANGE` → minor. To force a specific version, edit it in the release PR branch.

## Regular release

1. Make sure `main` contains everything you want to release and CI is green.
2. Actions → **Release PR** (`release-pr.yml`) → Run workflow on `main`. It opens or updates a PR that bumps the workspace version, the in-tree dependency versions in `[workspace.dependencies]`, `Cargo.lock`, and each crate's `CHANGELOG.md`.
3. CI does not start by itself on that PR (it was opened by `GITHUB_TOKEN`). Run Actions → **CI** → Run workflow on the `release-plz-*` branch. Edit the changelog on the branch if needed.
4. Merge the release PR.
5. Actions → **Release** (`release.yml`) → Run workflow on `main`.
6. Approve the pending deployment to the `crates-io` environment (you are the required reviewer).
7. The job gets a short-lived crates.io token via Trusted Publishing, publishes every crate whose version isn't on crates.io yet (in dependency order), and creates git tags + GitHub releases. Check crates.io and the Releases page.

Re-running **Release** is safe: already-published versions are skipped.

## Security model

- Only users with write access can dispatch the workflows.
- Publishing runs in the `crates-io` environment: required reviewer = me, deployments allowed only from `main`.
- crates.io trusts only repo `citizen-stig/remarkable-tablet-cli-rs`, workflow `release.yml`, environment `crates-io`. Renaming the workflow file or the environment breaks publishing until the Trusted Publishing config is updated on crates.io for each crate.
- No long-lived crates.io token is stored anywhere. The temporary token is revoked when the job ends.
- `release-plz/action` and `rust-lang/crates-io-auth-action` are pinned by commit SHA in `release.yml`; Dependabot bumps them. Review those bumps before merging, they see the publish token.

## Adding a new crate

1. Add it to the workspace with `version.workspace = true` and, if siblings depend on it, a `path` + `version` entry in `[workspace.dependencies]`.
2. Trusted Publishing only works for crates that already exist on crates.io, so publish its first version by hand: `cargo login`, `cargo publish -p <crate>`, then revoke the token.
3. On crates.io → crate Settings → Trusted Publishing, add the GitHub publisher (owner `citizen-stig`, repo `remarkable-tablet-cli-rs`, workflow `release.yml`, environment `crates-io`).

## One-time setup (not done yet as of 2026-11-10)

- First publish of 0.1.0 by hand from `main`: `cargo publish --workspace --dry-run`, then `cargo login` + `cargo publish --workspace`, then revoke the token.
- Trusted Publishing configured on crates.io for all four crates (values above).
- GitHub → Settings → Environments → `crates-io`: required reviewer = me, "Prevent self-review" off, deployment branches = `main`.
- GitHub → Settings → Actions → General → "Allow GitHub Actions to create and approve pull requests" on.

## Troubleshooting

- **Release job stuck "Waiting"**: approve the `crates-io` deployment on the run page.
- **Auth step fails**: Trusted Publishing config on crates.io doesn't match the workflow file name / environment / repo, or the crate isn't on crates.io yet.
- **Release PR not created**: the "Allow GitHub Actions to create and approve pull requests" setting is off.
- **Dry run locally**: `cargo publish --workspace --dry-run`.
