# Windows, Linux and macOS release-ref nightly: supervised manual go-live

**Enabled for supervised manual runs, not yet qualified.** The producer (`nightly.yml`, main) and
the signer (`nightly-sign.yml`, on the dedicated `nightly-signer` branch) keep only their exact
context guards and run when one of the reviewed manual actors dispatches them.
`nightly-dispatch.yml` stays literally off: it only forwards scheduled producers. There is no
schedule. The installed policy
[`.github/nightly-signing-policy.json`](../.github/nightly-signing-policy.json) is the v3 policy.
Its `producer_workflow_id` is the registration of `nightly.yml` on main, read with:

```bash
gh api repos/storytold/wordcraft/actions/workflows/nightly.yml --jq .id
```

The callers pass no platform secrets. The unsigned reusable caller's maximum OIDC/attestation
permissions are for its isolated attester; its compiler narrows to contents and Actions read
only, with no OIDC, attestation write, release environment or platform signing secrets.
Shared tooling is pinned to `be00e5ccd18b68e950c8a08532f2230e7c3d0cba`, the craft-common-actions
main commit that merged [#371](https://github.com/storytold/craft-common-actions/pull/371) (the
candidate is admitted on the built commit's own Sonar analysis, not main's latest) after
[#370](https://github.com/storytold/craft-common-actions/pull/370) (the Windows and macOS nightly
registry rows for the 14 apps) and
[#369](https://github.com/storytold/craft-common-actions/pull/369) (shared helper checkouts kept out
of the app's clean-source check, and macOS on `macos-15`), following
[#364](https://github.com/storytold/craft-common-actions/pull/364) (the signer may run on the
policy's signer branch) and [#363](https://github.com/storytold/craft-common-actions/pull/363) (the
multi-app pipeline, which makes the signer's `app-name` input required and admits any reviewed
registry app), with #342 (`a4f6fd51`, policy v3 and the `platforms` input) and #339 (`605d5876`)
earlier. Both callers name it in `uses` and `tooling-revision`. A pin is reviewed source
availability, not provider, queue, native signing or crypto qualification.

**Registry admission.** Craft-common-actions #363 ("run the shared nightly pipeline for any reviewed
registry app") is in the pin, and `nightly-apps.json` admits `storytold/wordcraft`. Both callers'
`uses`, `tooling-revision` and the policy pins move together to the same reviewed revision. The
signer also requires the policy's `platforms` map to equal the app's admitted targets, so
WordCraft's `windows-x86_64` and `macos-aarch64` rows need the targets and entrypoints that a
craft-common-actions follow-up adds; until it merges, the first signer run stops at admission.

**First runs.** The first runs are manual dispatches in this order: `nightly.yml` on main (only a
listed manual actor passes the producer's plan step), then `nightly-sign.yml` on `nightly-signer`
with that producer run ID and attempt, by a listed manual actor. `nightly-dispatch.yml` only matters
once a schedule is approved.

**Signer branch.** The signer runs on `refs/heads/nightly-signer`, not on the stable `release`
branch, so nightly signer files never need a stable release sync. The policy's `release_branch`
names it, the signer's guard requires it and the dispatcher targets it. The branch only
fast-forwards to reviewed main commits, through the settings script's `advance` (no queued signer
run, a trust-closure scan, then the full diff of `.github/` and `packaging/`), and the existing
`release` environment trusts it alongside `release`. Its trust closure is all of `.github/` and
`packaging/`, including `packaging/env.sh`, which `packaging/macos/import-cert.sh` sources with
the Apple keys. On that branch only the pinned `nightly-sign.yml` caller may reach an environment:
no app workflow binds one, nothing else calls `nightly-release.yml`, and the only caller of the
shared stable `release.yml` is `release.yml`, whose callee must carry the stable-only environment
guard. The contract tests check the in-repo part on every main PR; the settings scan also resolves
the callee. Stable releases still come only from `refs/heads/release`. History does not carry over
a switch between `release` and `nightly-signer`, so the switch happens before the first
publication.

**Registry admission.** The signer requires the protected policy's `platforms` map to equal the
run app's admitted targets in craft-common-actions' `nightly-apps.json`, and it refuses rather
than sign a subset. WordCraft's row admits `linux-x86_64`; the matching `windows-x86_64` and
`macos-aarch64` targets and entrypoints are a craft-common-actions follow-up, and the first
signer run stops at admission until they merge.

**Manual actors.** `bflatastic`, `dylanmtaylor`, `echelon`, `FredApps` and `skylartaylor`, in the
producer's `manual-actors` and the policy's `manual_actors`. The list mirrors the release-managers
team (id 19837241), leaving out `echelon-robot`; update it by PR in both places when the team
changes.

**Admission timing.** The candidate check reads the newest CI run on the candidate commit and
main's latest finished Sonar analysis, so pick a quiet main, wait for its CI and SonarCloud analysis
to finish green, then dispatch the producer and, once it succeeds, the signer, before another main
push lands a newer analysis.

**Platform rows.** The producer caller lists
`platforms: '["windows-x86_64","linux-x86_64","macos-aarch64"]'` on its single job: one producer
run builds every listed platform for one all-or-nothing release (one failed or skipped platform stops
that night's release), and the signer admits platforms only from the protected policy, so
there is no separate Linux or macOS caller job and the signer caller has no platform input.

## Installed policy values and their sources

Read on 2026-11-24; re-read anything that may have moved (and fill in `producer_workflow_id` after
`nightly.yml` lands on `main`) before `nightly-signer` is advanced to it.

| Field | Value | Source |
|---|---|---|
| `repository_id` | `1408489772` | `gh api repos/storytold/wordcraft --jq .id` |
| `producer_workflow_id` | `429734489` | `gh api repos/storytold/wordcraft/actions/workflows/nightly.yml --jq .id` |
| `producer_tooling_revision`, `signer_tooling_revision`, `producer_workflow_ref` | `be00e5cc…` (craft-common-actions #371 after #370, #369 and #364) | The callers' pin |
| `manual_actors` / `automatic_signer_actors` | the five release managers / `[]` | release-managers team (id 19837241) members; equals the producer caller's `manual-actors` |
| `max_age_seconds`, `max_artifact_bytes` | `86400`, `2147483648` | Go-live checklist |
| `platforms` | three rows | Storytold stable signing identities (Windows Authenticode subject, Linux OpenPGP fingerprint, macOS Apple Team ID `DJ6XS33FX8`) |
| `candidate_policy.required_workflows` | CI (`ci.yml`, workflow `380135788`), `gate / gate (ubuntu-24.04)` | Workflow list API; always-run main push gate job |
| `candidate_policy.sonar` | `storytold` / `storytold_wordcraft`, app `12526` | `sonar-project.properties` |
| Sonar conditions | `new_security_rating` accepted (max 5); the other five required | Standard Storytold SonarCloud quality gate |
| Sonar `issue_types` / `debt` | `VULNERABILITY`; `[]` | Refresh from SonarCloud before `nightly-signer` is advanced to it |

The candidate check refuses whenever main's unresolved vulnerability set differs from `debt`, whether
an issue is new or fixed, and whenever a required condition fails or a listed CI job is renamed. Refresh
those values in the same reviewed change that `nightly-signer` is then advanced to.

## Ownership and interface

- `.github/workflows/nightly.yml`: default-main registration of the unsigned producer, named
  `WordCraft unsigned nightly`, enabled for manual dispatch by `manual-actors`; no signing secrets
  may be passed or inherited.
- `.github/workflows/nightly-sign.yml`: top-level registration on `nightly-signer`, enabled for manual dispatch,
  and only ever fast-forwards to reviewed main commits. It rejects wrong repository/event/actual
  branch, including same-name tags. Its constant whole-run concurrency (`wordcraft-nightly-release`,
  `queue: max`, `cancel-in-progress: false`) is distinct from stable. The caller itself passes no
  credentials; only the shared callee's sign jobs bind the `release` environment, which trusts `release` and
  `nightly-signer`.
- `.github/workflows/nightly-dispatch.yml`: transport only, never authorization. It is literally
  off (`false &&`) and forwards only a first-attempt scheduled producer to `nightly-sign.yml` on
  `nightly-signer`. No schedule is registered until activation is reviewed.
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
- The signer guards `refs/heads/nightly-signer`, passes `app-name: wordcraft`, and the policy's
  `release_branch` names it.
- A trust-closure scan reads every workflow: nothing binds an `environment`, only `nightly-sign.yml`
  calls `nightly-release.yml` (at a full SHA), and the only caller of the shared stable `release.yml`
  is `release.yml`.
- `nightly-dispatch.yml` keeps a literal-off guard, a `workflow_run`-only trigger and a
  `refs/heads/nightly-signer` request.
