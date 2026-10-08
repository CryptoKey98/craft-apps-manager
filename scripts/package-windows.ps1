param(
    [Parameter(Mandatory)][ValidateSet('x64', 'x86')][string]$Architecture,
    [Parameter(Mandatory)][string]$SevenZipFolder,
    [string]$TargetFolder = $env:CARGO_TARGET_DIR,
    [string]$OutputFolder
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$target = if ($Architecture -eq 'x86') { 'i686-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
$machine = if ($Architecture -eq 'x86') { 0x14c } else { 0x8664 }
function Assert-Machine([string]$Path) {
    $bytes = [IO.File]::ReadAllBytes($Path)
    $offset = [BitConverter]::ToInt32($bytes, 0x3c)
    if ([BitConverter]::ToUInt16($bytes, $offset + 4) -ne $machine) {
        throw "Wrong architecture for $Path; expected $Architecture"
    }
}
if (!$TargetFolder) { $TargetFolder = Join-Path $projectRoot 'target' }
$exe = Join-Path $TargetFolder "$target/release/craft-apps-manager.exe"
$seven = Join-Path $SevenZipFolder '7za.exe'
Assert-Machine $exe
Assert-Machine $seven
$version = [regex]::Match((Get-Content (Join-Path $projectRoot 'Cargo.toml') -Raw), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
$output = if ($OutputFolder) { $OutputFolder } else { Join-Path $projectRoot "dist/windows-$Architecture" }
$appFolder = Join-Path $output 'Craft Apps Manager'
if (Test-Path -LiteralPath $appFolder) { throw "Package folder already exists: $appFolder" }
$toolsFolder = Join-Path $appFolder "workspace/tools/7zip/$Architecture"
New-Item -ItemType Directory -Path $toolsFolder -Force | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $appFolder 'CraftApps-Manager.exe')
foreach ($name in @('README.md', 'CHANGELOG.md', 'LICENSE', 'THIRD-PARTY-NOTICES.txt')) {
    Copy-Item -LiteralPath (Join-Path $projectRoot $name) -Destination $appFolder
}
$licenses = Join-Path $appFolder 'licenses/app-icons'
New-Item -ItemType Directory -Path $licenses -Force | Out-Null
Get-ChildItem -LiteralPath (Join-Path $projectRoot 'assets/app-icons') -Filter '*.txt' | Copy-Item -Destination $licenses
Copy-Item -LiteralPath $seven -Destination $toolsFolder
# License notices can sit in the parent SDK directory for its x64 binary.
$noticeFolder = if (Test-Path -LiteralPath (Join-Path $SevenZipFolder 'License.txt')) { $SevenZipFolder } else { Split-Path -Parent $SevenZipFolder }
foreach ($name in @('License.txt', 'readme.txt', 'history.txt')) {
    Copy-Item -LiteralPath (Join-Path $noticeFolder $name) -Destination $toolsFolder
}
$archive = Join-Path $output "Craft-Apps-Manager-$version-windows-$Architecture.zip"
Compress-Archive -LiteralPath $appFolder -DestinationPath $archive
$hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content -LiteralPath (Join-Path $output 'SHA256SUMS.txt') -Value "$hash  $(Split-Path -Leaf $archive)" -Encoding ascii
Write-Output $archive
