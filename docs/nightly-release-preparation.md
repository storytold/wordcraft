# Windows, Linux and macOS release-ref nightly: supervised manual go-live

**Enabled for supervised manual runs, not yet qualified.** The producer (`nightly.yml`, main) and the
signer (`nightly-sign.yml`, release) keep only their exact context guards and run when one of the reviewed
manual actors dispatches them. There is no schedule yet. The installed policy
[`.github/nightly-signing-policy.json`](../.github/nightly-signing-policy.json) is the v3 policy.
Once `nightly.yml` lands on `main`, populate `producer_workflow_id` before promoting the policy to
`release` by reading:

```bash
gh api repos/storytold/wordcraft/actions/workflows/nightly.yml --jq .id
```

The callers pass no platform secrets. The unsigned reusable caller's maximum OIDC/attestation
permissions are for its isolated attester; its compiler narrows to contents and Actions read
only, with no OIDC, attestation write, release environment or platform signing secrets.
Shared tooling is pinned to `605d58766e8f536564da8fc81d3676ac86816dea`, the craft-common-actions
main commit that merged [#339](https://github.com/storytold/craft-common-actions/pull/339) (bounded
lineage compares and an issue reread at the end of candidate admission) on top of
[#342](https://github.com/storytold/craft-common-actions/pull/342) (policy v3, the `platforms` input
and the Linux and macOS legs). Both callers name it in `uses` and `tooling-revision`. A pin is reviewed
source availability, not provider, queue, native signing or crypto qualification.
Existing stable workflows are unchanged.

**The shared-workflow port is a prerequisite.** At this pin both reusable workflows admit only
PhotoCraft: `nightly-unsigned.yml` requires `inputs.app-name == 'photocraft'` and
`github.repository == 'storytold/photocraft'` on its build jobs, and `nightly-release.yml` requires the
same repository in `validate` and every signing job (its concurrency group is
`craft-release-ref-signer-photocraft`). A WordCraft dispatch therefore builds and signs nothing until
`storytold/craft-common-actions` admits `storytold/wordcraft`. No dispatch can produce a release before
that port, and both callers' `uses`, `tooling-revision` and the policy pins must move to the same
reviewed revision together. The port is craft-common-actions #363 ("run the shared nightly pipeline for any
reviewed registry app"); until it merges these callers are inert.

**First runs.** The first runs are manual dispatches in this order: `nightly.yml` on main (only a
listed manual actor passes the producer's plan step), then `nightly-sign.yml` on `release` with that
producer run ID and attempt, by a listed manual actor.

**Manual actors.** `bflatastic`, `dylanmtaylor`, `echelon`, `FredApps` and `skylartaylor`, in the
producer's `manual-actors` and the policy's `manual_actors`. The list mirrors the release-managers
team (id 19837241), leaving out `echelon-robot`; update it by PR in both places when the team changes.

**Admission timing.** The candidate check reads the newest CI run on the candidate commit and
main's latest finished Sonar analysis, so pick a quiet main, wait for its CI and SonarCloud analysis
to finish green, then dispatch the producer and, once it succeeds, the signer, before another main
push lands a newer analysis.

**Platform rows.** The producer caller lists
`platforms: '["windows-x86_64","linux-x86_64","macos-aarch64"]'` on its single job: one producer
run builds every listed platform for one all-or-nothing release (one failed or skipped platform stops
that night's release), and the release-ref signer admits platforms only from the protected policy, so
there is no separate Linux or macOS caller job and the signer caller has no platform input.

## Installed policy values and their sources

Read on 2026-11-24; re-read anything that may have moved (and fill in `producer_workflow_id` after
`nightly.yml` lands on `main`) before the policy is promoted to `release`.

| Field | Value | Source |
|---|---|---|
| `repository_id` | `1408489772` | `gh api repos/storytold/wordcraft --jq .id` |
| `producer_workflow_id` | `null` (pending `main` registration) | Read via `gh api repos/storytold/wordcraft/actions/workflows/nightly.yml --jq .id` once `nightly.yml` reaches `main` |
| `producer_tooling_revision`, `signer_tooling_revision`, `producer_workflow_ref` | `605d5876…` (craft-common-actions #339) | The callers' pin |
| `manual_actors` / `automatic_signer_actors` | the five release managers / `[]` | release-managers team (id 19837241) members; equals the producer caller's `manual-actors` |
| `max_age_seconds`, `max_artifact_bytes` | `86400`, `2147483648` | Go-live checklist |
| `platforms` | three rows | Storytold stable signing identities (Windows Authenticode subject, Linux OpenPGP fingerprint, macOS Apple Team ID `DJ6XS33FX8`) |
| `candidate_policy.required_workflows` | CI (`ci.yml`, workflow `380135788`), `gate / gate (ubuntu-24.04)` | Workflow list API; always-run main push gate job |
| `candidate_policy.sonar` | `storytold` / `storytold_wordcraft`, app `12526` | `sonar-project.properties` |
| Sonar conditions | `new_security_rating` accepted (max 5); the other five required | Standard Storytold SonarCloud quality gate |
| Sonar `issue_types` / `debt` | `VULNERABILITY`; `[]` | Refresh from SonarCloud before promoting policy to `release` |

The candidate check refuses whenever main's unresolved vulnerability set differs from `debt`, whether
an issue is new or fixed, and whenever a required condition fails or a listed CI job is renamed. Refresh
those values in the same reviewed change that promotes the policy to `release`.

## Ownership and interface

- `.github/workflows/nightly.yml`: default-main registration of the unsigned producer, named
  `WordCraft unsigned nightly`, enabled for manual dispatch by `manual-actors`; no signing secrets
  may be passed or inherited.
- `.github/workflows/nightly-sign.yml`: top-level release registration, enabled for manual dispatch,
  and identical on main and release after reviewed promotion. It rejects wrong repository/event/actual
  branch, including same-name tags. Its constant whole-run concurrency (`wordcraft-nightly-release`,
  `queue: max`, `cancel-in-progress: false`) is distinct from stable. The caller itself passes no
  credentials; only the shared callee's sign jobs bind the `release` environment.
- Shared reusable entrypoints are `nightly-unsigned.yml` and `nightly-release.yml` in
  `storytold/craft-common-actions`. Both must resolve to a **real reviewed public immutable commit**;
  the defining workflow SHA and tooling input must match.
- `packaging/windows/sign.ps1 -Required` is the app-owned Windows adapter, invoked only from the
  immutable release execution SHA checkout.

## Local regression checks

`python3 packaging/nightly-release-tests.py` verifies:
- Both callers share one full 40-char commit pin in `uses` and `tooling-revision`.
- Both callers keep their exact context guards (reading all continuation lines), have no cache
  actions/options or secrets, and register no schedule.
- `nightly-sign.yml` keeps its literal `wordcraft-nightly-release` concurrency group, `queue: max`,
  and `cancel-in-progress: false`.
- `manual-actors` in `nightly.yml` and `manual_actors` in `.github/nightly-signing-policy.json` are
  identical and equal to the reviewed release managers list, with `automatic_signer_actors` empty.
- `.github/nightly-signing-policy.json` matches the `craft-nightly-release-policy/v3` schema,
  repository identity, platform trust anchors, CI workflow trigger, and `sonar-project.properties`.
