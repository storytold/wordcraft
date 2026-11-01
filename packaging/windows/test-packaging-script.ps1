<# Check the real packaging script without compiling, signing or installing: one MSI, policy-checked before it is signed. #>
$ErrorActionPreference = 'Stop'
$tokens = $null; $errors = $null
$script = Join-Path $PSScriptRoot 'package.ps1'
$ast = [Management.Automation.Language.Parser]::ParseFile($script, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'The packaging script does not parse.' }
if ($ast.Extent.Text -match 'windows-user|UserMsi') {
  throw 'WordCraft ships one dual-scope MSI; there is no separate per-user installer to package.'
}
$commands = @($ast.FindAll({ param($node) $node -is [Management.Automation.Language.CommandAst] }, $true))
$checks = @($commands | Where-Object {
  $_.InvocationOperator.ToString() -eq 'Ampersand' -and
    @($_.CommandElements | Where-Object { $_.Extent.Text -match 'check-install-policy.ps1' }).Count -gt 0
})
if ($checks.Count -ne 1) { throw 'Expected exactly one policy check in the packaging script.' }
$elements = $checks[0].CommandElements
$index = 0
while ($index -lt $elements.Count -and $elements[$index].Extent.Text -ne '-MsiPath') { $index++ }
if ($index + 1 -ge $elements.Count -or $elements[$index + 1].Extent.Text -ne '$Msi') {
  throw 'The policy check must inspect the MSI being packaged ($Msi).'
}
$signs = @($commands | Where-Object {
  $_.InvocationOperator.ToString() -eq 'Ampersand' -and
    @($_.CommandElements | Where-Object { $_.Extent.Text -match 'sign.ps1' }).Count -gt 0 -and
    $_.CommandElements[-1].Extent.Text -eq '$Msi'
})
if ($signs.Count -ne 1) { throw 'Expected the MSI to be signed exactly once.' }
if ($checks[0].Extent.StartOffset -ge $signs[0].Extent.StartOffset) { throw 'The MSI must pass the policy check before it is signed.' }
Write-Output 'PASS the WordCraft MSI is policy-checked before it is signed.'
