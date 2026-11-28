# Windows packaging

See [installation policy checks](INSTALLATION.md) for the installer scope and path gates.

## Nightly build and signing boundary

`package.ps1 -Arch x64 -Nightly -BuildOnly` is for the secret-free compilation job.
It refuses signing credential variables and builds the GUI only, without CLI,
portable marker, installer or associations. Its `*-nightly.unsigned.zip` is an
internal intermediate, not the final feed artifact. There is no unsigned
publication switch.

The separate trusted producer signing job safely unpacks that new intermediate,
checks its exact identity/inventory, then invokes the existing backend:

```powershell
pwsh packaging/windows/sign.ps1 -Required -CheckOnly
pwsh packaging/windows/sign.ps1 -Required -Files <stage>/wordcraft.exe
```

Missing or partial configuration fails. `-Required` verifies Authenticode and
a signer certificate after the existing signtool verification. Stable local
packaging keeps its existing optional-signing policy. The producer independently
verifies the final signature, PE identity and inventory before archiving under
the normal feed filename, and refuses to overwrite prepared outputs. It never
executes the payload in the signing step. Real keys belong only to that step,
not compilation. These interfaces do not configure or grant access to keys.

The signing-policy fixtures use generated dummy values and a mocked compiler
boundary; they do not prove real certificate signing or release readiness.
