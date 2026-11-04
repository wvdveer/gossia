<#
.SYNOPSIS
  Builds the MSIX package of ClioTree for the Microsoft Store, on Windows.

.DESCRIPTION
  Builds the release cliotree.exe and cliotree-cli.exe, writes the logos
  (cargo xtask icons), fills in AppxManifest.xml and packs it all with MakeAppx
  from the Windows SDK. The package is not signed: the Store signs it when it is
  uploaded in Partner Center (DESIGN.md, Testing and CI).

  The result is target\msix\ClioTree-<version>-x64.msix, and the folder it was
  packed from is target\msix\stage.

.PARAMETER SkipBuild
  Use the release executables already in target\release.

.PARAMETER Register
  Install the packed folder for this user (Add-AppxPackage -Register), to try
  the package before uploading it. Needs Developer Mode turned on in Settings.
  Remove it again with: Get-AppxPackage WardvanderVeer.ClioTree | Remove-AppxPackage

.PARAMETER Publish
  Upload the .msix to the generic package registry of the Gitea instance
  (http://acer.patersonst:3000, package cliotree, owner ward), replacing a file
  of the same version. The login is the one Git Credential Manager holds for
  that host and user ward; the token needs the package write permission.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File packaging\windows\build-msix.ps1 -Register
#>
param(
    [switch]$SkipBuild,
    [switch]$Register,
    [switch]$Publish
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $root

$cargoToml = Get-Content (Join-Path $root 'Cargo.toml') -Raw
if ($cargoToml -notmatch '(?m)^version = "(\d+)\.(\d+)\.(\d+)"') {
    throw 'no version in Cargo.toml'
}
$version = "$($Matches[1]).$($Matches[2]).$($Matches[3])"
# The Store wants four parts, the last 0.
$packageVersion = "$version.0"

function Invoke-Native([string]$program, [string[]]$arguments) {
    Write-Host "+ $program $($arguments -join ' ')"
    & $program @arguments
    if ($LASTEXITCODE -ne 0) { throw "$program failed (exit $LASTEXITCODE)" }
}

if (-not $SkipBuild) {
    Invoke-Native 'cargo' @('build', '--release', '-p', 'cliotree-app', '-p', 'cliotree-cli',
        '--bin', 'cliotree', '--bin', 'cliotree-cli')
}

$out = Join-Path $root 'target\msix'
$stage = Join-Path $out 'stage'
$icons = Join-Path $out 'icons'
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force (Join-Path $stage 'Assets') | Out-Null
Invoke-Native 'cargo' @('xtask', 'icons', $icons)

foreach ($exe in 'cliotree.exe', 'cliotree-cli.exe') {
    $from = Join-Path $root "target\release\$exe"
    if (-not (Test-Path $from)) { throw "$from is missing: build without -SkipBuild" }
    Copy-Item $from $stage
}
Copy-Item (Join-Path $root 'LICENSE') (Join-Path $stage 'LICENSE.txt')

# The logos: the names the manifest gives, and for the 44-pixel logo the sizes
# Windows picks for the taskbar, Start and Explorer (unplated: without the
# coloured square behind them).
$assets = Join-Path $stage 'Assets'
Copy-Item (Join-Path $icons '44.png') (Join-Path $assets 'Square44x44Logo.png')
Copy-Item (Join-Path $icons '150.png') (Join-Path $assets 'Square150x150Logo.png')
Copy-Item (Join-Path $icons '50.png') (Join-Path $assets 'StoreLogo.png')
foreach ($size in 16, 24, 32, 48, 256) {
    $png = Join-Path $icons "$size.png"
    Copy-Item $png (Join-Path $assets "Square44x44Logo.targetsize-$size.png")
    Copy-Item $png (Join-Path $assets "Square44x44Logo.targetsize-${size}_altform-unplated.png")
}

$manifest = (Get-Content (Join-Path $PSScriptRoot 'AppxManifest.xml') -Raw).Replace('@VERSION@', $packageVersion)
[System.IO.File]::WriteAllText((Join-Path $stage 'AppxManifest.xml'), $manifest, (New-Object System.Text.UTF8Encoding $false))

# MakeAppx from the newest Windows SDK installed.
$makeappx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\makeappx.exe" -ErrorAction SilentlyContinue |
    Sort-Object { [version]$_.Directory.Parent.Name } -Descending | Select-Object -First 1
if (-not $makeappx) { throw 'makeappx.exe not found: install the Windows SDK' }

$msix = Join-Path $out "ClioTree-$version-x64.msix"
Invoke-Native $makeappx.FullName @('pack', '/o', '/d', $stage, '/p', $msix)
Write-Host "Packed $msix"

if ($Register) {
    Add-AppxPackage -Register (Join-Path $stage 'AppxManifest.xml')
    Write-Host 'Registered for this user: ClioTree is on the Start menu, and cliotree-cli runs by name.'
}

if ($Publish) {
    $server = 'http://acer.patersonst:3000'
    $query = "protocol=http`nhost=acer.patersonst:3000`nusername=ward`n`n"
    $env:GCM_INTERACTIVE = 'never'
    $answer = $query | git credential fill
    $password = ($answer | Where-Object { $_ -like 'password=*' }) -replace '^password=', ''
    if (-not $password) { throw 'Git Credential Manager has no login for ward at acer.patersonst:3000' }
    $basic = [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes("ward:$password"))
    $headers = @{ Authorization = "Basic $basic" }
    $url = "$server/api/packages/ward/generic/cliotree/$version/$(Split-Path $msix -Leaf)"
    try {
        Invoke-WebRequest -UseBasicParsing -Method Delete -Uri $url -Headers $headers | Out-Null
        Write-Host 'Removed the earlier file of this version'
    } catch {
        # 404: there was none.
    }
    Invoke-WebRequest -UseBasicParsing -Method Put -Uri $url -Headers $headers -InFile $msix | Out-Null
    Write-Host "Published $url"
}
