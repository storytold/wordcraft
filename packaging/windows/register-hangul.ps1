<#
.SYNOPSIS
  Register a WordCraft build for Hangul documents (.hwpx, .hwp) for the current user, without
  an installer or administrator rights; -Remove takes it out again.
.DESCRIPTION
  For trying a local build from Explorer. Writes only under HKCU\Software:
  - ProgIds WordCraft.Local.HangulDocument (.hwpx) and WordCraft.Local.LegacyHangulDocument
    (.hwp), with their icon and open command. They are not the installer's ProgIds
    (wordcraft.wxs), so a per-user installation keeps its own, and -Remove leaves it working;
  - the extensions' OpenWithProgids, so the build is listed under Explorer's "Open with";
  - a "WordCraft" entry on the files' context menu (SystemFileAssociations), so one click opens
    a file in the build whichever app is the default;
  - Capabilities under RegisteredApplications, so Settings > Default apps lists it, where the
    user can make it the default. Nothing here changes an extension's default.
.EXAMPLE
  .\register-hangul.ps1 -Exe "D:\builds\wordcraft\release\wordcraft.exe"
.EXAMPLE
  .\register-hangul.ps1 -Remove
#>
param(
  # the release build in CARGO_TARGET_DIR, or in the repository's target folder
  [string] $Exe = (Join-Path $(if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PSScriptRoot '..\..\target' }) 'release\wordcraft.exe'),
  [switch] $Remove
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$classes = 'HKCU:\Software\Classes'
$appKey = 'HKCU:\Software\WordCraftHangul'
$appName = 'WordCraft (Hangul)'
# "WordCraft<ro> <yeol-gi>" (Open in WordCraft), from code points so the script stays ASCII
$verb = 'WordCraft' + [char]0xB85C + ' ' + [char]0xC5F4 + [char]0xAE30
$types = @(
  @{ Ext = '.hwpx'; ProgId = 'WordCraft.Local.HangulDocument'; Name = 'Hangul Document (HWPX)' },
  @{ Ext = '.hwp'; ProgId = 'WordCraft.Local.LegacyHangulDocument'; Name = 'Hangul Document (HWP)' }
)

function Set-Value([string] $Key, [string] $Name, [string] $Value) {
  if (-not (Test-Path -LiteralPath $Key)) { New-Item -Path $Key -Force | Out-Null }
  if ($Name -eq '') { Set-Item -LiteralPath $Key -Value $Value } else { New-ItemProperty -LiteralPath $Key -Name $Name -Value $Value -PropertyType String -Force | Out-Null }
}

function Remove-Key([string] $Key) {
  if (Test-Path -LiteralPath $Key) { Remove-Item -LiteralPath $Key -Recurse -Force }
}

function Remove-Value([string] $Key, [string] $Name) {
  if ((Test-Path -LiteralPath $Key) -and $null -ne (Get-ItemProperty -LiteralPath $Key -Name $Name -ErrorAction SilentlyContinue)) {
    Remove-ItemProperty -LiteralPath $Key -Name $Name
  }
}

if ($Remove) {
  foreach ($t in $types) {
    Remove-Key "$classes\$($t.ProgId)"
    Remove-Value "$classes\$($t.Ext)\OpenWithProgids" $t.ProgId
    Remove-Key "$classes\SystemFileAssociations\$($t.Ext)\shell\WordCraft"
  }
  Remove-Value 'HKCU:\Software\RegisteredApplications' $appName
  Remove-Key $appKey
} else {
  if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) { throw "no WordCraft build at $Exe (build it with: cargo build --release -p wordcraft)" }
  $Exe = (Resolve-Path -LiteralPath $Exe).Path
  $open = '"' + $Exe + '" "%1"'
  $icon = '"' + $Exe + '",0'
  Set-Value $appKey 'InstallDir' (Split-Path -Parent $Exe)
  Set-Value "$appKey\Capabilities" 'ApplicationName' $appName
  Set-Value "$appKey\Capabilities" 'ApplicationDescription' 'Open, edit and save Hangul documents (HWPX, HWP).'
  Set-Value "$appKey\Capabilities" 'ApplicationIcon' $icon
  Set-Value 'HKCU:\Software\RegisteredApplications' $appName 'Software\WordCraftHangul\Capabilities'
  foreach ($t in $types) {
    $p = "$classes\$($t.ProgId)"
    Set-Value $p '' $t.Name
    Set-Value "$p\DefaultIcon" '' $icon
    Set-Value "$p\Application" 'ApplicationName' $appName
    Set-Value "$p\shell\open\command" '' $open
    Set-Value "$classes\$($t.Ext)\OpenWithProgids" $t.ProgId ''
    $menu = "$classes\SystemFileAssociations\$($t.Ext)\shell\WordCraft"
    Set-Value $menu 'MUIVerb' $verb
    Set-Value $menu 'Icon' $icon
    Set-Value "$menu\command" '' $open
    Set-Value "$appKey\Capabilities\FileAssociations" $t.Ext $t.ProgId
  }
}

# Tell Explorer the associations changed (SHCNE_ASSOCCHANGED), so menus and icons update now.
Add-Type -Namespace WordCraft -Name Shell -MemberDefinition '[DllImport("shell32.dll")] public static extern void SHChangeNotify(int e, uint f, IntPtr a, IntPtr b);'
[WordCraft.Shell]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)
if ($Remove) { 'Removed the Hangul document registration.' } else { "Registered $Exe for .hwpx and .hwp." }
