# Windows 7 x64 import compatibility

Upstream: Microsoft windows-link 0.2.1, from crates.io, MIT OR Apache-2.0.
Source: https://crates.io/crates/windows-link/0.2.1
The original license texts are retained here. The same patch is carried by VectorCraft's
Windows 7 build (storytold/vectorcraft `vendor/windows-link`).

Only the non-x86 win7-target macro differs: CoTaskMemFree is imported from
ole32.dll rather than combase.dll. windows-sys 0.61.2 otherwise causes native
file dialogs to introduce a DLL absent on Windows 7. Ordinary targets retain
upstream behavior. This is a binding correction, not a replacement allocator.

Microsoft documents Ole32.dll and Windows 2000 as the API's minimum:
https://learn.microsoft.com/en-us/windows/win32/api/combaseapi/nf-combaseapi-cotaskmemfree

Only the experimental Windows 7 build uses this copy: packaging/windows/windows7.ps1
passes `--config "patch.crates-io.windows-link.path='vendor/windows-link'"`. The
workspace has no windows-link entry in `[patch]`, so every other build uses the crates.io
crate, verified by the checksum in Cargo.lock.

The Windows 7 packaging check rejects combase.dll to guard this fix. Remove this
patch when upstream bindings provide the same compatibility and recheck imports.
