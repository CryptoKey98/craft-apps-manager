param(
    [Parameter(Mandatory)][ValidateSet('x64', 'x86')][string]$Architecture,
    [Parameter(Mandatory)][string]$SevenZipFolder,
    [string]$Wix = 'wix',
    [string]$TargetFolder = $env:CARGO_TARGET_DIR,
    [string]$OutputFolder
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
if (!$TargetFolder) { $TargetFolder = Join-Path $projectRoot 'target' }
if (!$OutputFolder) { $OutputFolder = Join-Path $projectRoot "dist/windows-$Architecture" }
$target = if ($Architecture -eq 'x86') { 'i686-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
$machine = if ($Architecture -eq 'x86') { 0x14c } else { 0x8664 }
$exe = Join-Path $TargetFolder "$target/release/craft-apps-manager.exe"
$seven = Join-Path $SevenZipFolder '7za.exe'
foreach ($path in @($exe, $seven)) {
    $bytes = [IO.File]::ReadAllBytes($path)
    $offset = [BitConverter]::ToInt32($bytes, 0x3c)
    if ([BitConverter]::ToUInt16($bytes, $offset + 4) -ne $machine) { throw "Wrong architecture: $path" }
}
$version = [regex]::Match([IO.File]::ReadAllText((Join-Path $projectRoot 'Cargo.toml')), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
$stage = Join-Path $OutputFolder "msi-payload-$version"
if (Test-Path -LiteralPath $stage) { throw "Package staging folder already exists: $stage" }
# DirectoryInfo.FullName normalizes relative output paths before deriving stable
# component identities from paths relative to the payload root.
$stage = (New-Item -ItemType Directory -Path $stage -Force).FullName
Copy-Item -LiteralPath $exe -Destination (Join-Path $stage 'CraftApps-Manager.exe')
Set-Content -LiteralPath (Join-Path $stage 'installation-msi.json') -Value '{"format":"msi"}' -Encoding ascii
foreach ($name in @('README.md','CHANGELOG.md','LICENSE','THIRD-PARTY-NOTICES.txt')) {
    Copy-Item -LiteralPath (Join-Path $projectRoot $name) -Destination $stage
}
$licenses = Join-Path $stage 'licenses/app-icons'
$sevenStage = Join-Path $stage "tools/7zip/$Architecture"
New-Item -ItemType Directory -Path $licenses,$sevenStage -Force | Out-Null
Get-ChildItem -LiteralPath (Join-Path $projectRoot 'assets/app-icons') -Filter '*.txt' | Copy-Item -Destination $licenses
Copy-Item -LiteralPath $seven -Destination $sevenStage
$noticeFolder = if (Test-Path -LiteralPath (Join-Path $SevenZipFolder 'License.txt')) { $SevenZipFolder } else { Split-Path -Parent $SevenZipFolder }
foreach ($name in @('License.txt','readme.txt','history.txt')) {
    Copy-Item -LiteralPath (Join-Path $noticeFolder $name) -Destination $sevenStage
}
function Escape-Xml([string]$Text) { [Security.SecurityElement]::Escape($Text) }
# Stable and distinct upgrade families let x86 and x64 coexist without replacing one another.
$upgrade = if ($Architecture -eq 'x86') { '706B76C5-B377-48B9-AC61-E65986704A2F' } else { '964F2264-6758-4DE9-BBE9-1C74B295E47B' }
$components = [Collections.Generic.List[string]]::new()
$counter = 0
function Write-PayloadDirectory([string]$Path) {
    $xml = ''
    $first = $true
    foreach ($file in Get-ChildItem -LiteralPath $Path -File | Sort-Object Name) {
        $script:counter++
        $relative = $file.FullName.Substring($stage.Length + 1)
        $digest = [Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes("$Architecture/$relative"))
        $id = 'Payload' + ([BitConverter]::ToString($digest).Replace('-','').Substring(0,24))
        $guid = [Guid]::new([byte[]]$digest[0..15]).ToString()
        $remove = if ($first) { "<RemoveFolder Id='Remove$id' On='uninstall'/>" } else { '' }
        $first = $false
        $components.Add($id)
        $xml += "<Component Id='$id' Guid='$guid'><File Id='File$id' Source='$(Escape-Xml $file.FullName)'/>$remove<RegistryValue Root='HKCU' Key='Software\CraftAppsManager\$Architecture\Files' Name='$(Escape-Xml $relative)' Type='integer' Value='1' KeyPath='yes'/></Component>"
    }
    foreach ($folder in Get-ChildItem -LiteralPath $Path -Directory | Sort-Object Name) {
        $script:counter++
        $xml += "<Directory Id='Folder$script:counter' Name='$(Escape-Xml $folder.Name)'>$(Write-PayloadDirectory $folder.FullName)</Directory>"
    }
    return $xml
}
$payload = Write-PayloadDirectory $stage
$references = ($components | ForEach-Object { "<ComponentRef Id='$_'/>" }) -join ''
$icon = Escape-Xml (Join-Path $projectRoot 'assets/icon.ico')
# Remove the previous product inside the rollback transaction, before copying
# new files. Older packages may have derived different component identities.
$xml = @"
<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs" xmlns:ui="http://wixtoolset.org/schemas/v4/wxs/ui">
  <Package Name="Craft Apps Manager ($Architecture)" Manufacturer="CryptoKey98" Version="$version" UpgradeCode="$upgrade" Scope="perUser">
    <MajorUpgrade DowngradeErrorMessage="A newer version of Craft Apps Manager is already installed." Schedule="afterInstallInitialize"/>
    <MediaTemplate EmbedCab="yes" CompressionLevel="high"/>
    <Icon Id="ManagerIcon" SourceFile="$icon"/>
    <Property Id="ARPPRODUCTICON" Value="ManagerIcon"/>
    <Property Id="ARPURLINFOABOUT" Value="https://github.com/CryptoKey98/craft-apps-manager"/>
    <StandardDirectory Id="LocalAppDataFolder"><Directory Id="ProgramsFolder" Name="Programs"><Directory Id="INSTALLFOLDER" Name="Craft Apps Manager $Architecture">$payload</Directory></Directory></StandardDirectory>
    <StandardDirectory Id="ProgramMenuFolder"><Directory Id="MenuFolder" Name="Craft Apps Manager $Architecture">
      <Component Id="Shortcuts" Guid="*">
        <Shortcut Id="ManagerShortcut" Name="Craft Apps Manager ($Architecture)" Target="[INSTALLFOLDER]CraftApps-Manager.exe" WorkingDirectory="INSTALLFOLDER" Icon="ManagerIcon"/>
        <Shortcut Id="BuilderShortcut" Name="Craft Apps Builder ($Architecture)" Target="[INSTALLFOLDER]CraftApps-Manager.exe" Arguments="--builder" WorkingDirectory="INSTALLFOLDER" Icon="ManagerIcon"/>
        <RemoveFolder Id="RemoveMenuFolder" On="uninstall"/>
        <RegistryValue Root="HKCU" Key="Software\CraftAppsManager\$Architecture" Name="Installed" Type="integer" Value="1" KeyPath="yes"/>
      </Component>
    </Directory></StandardDirectory>
    <Feature Id="Manager" Title="Craft Apps Manager" Level="1">$references<ComponentRef Id="Shortcuts"/></Feature>
    <ui:WixUI Id="WixUI_Minimal"/>
    <WixVariable Id="WixUILicenseRtf" Value="$(Escape-Xml (Join-Path $projectRoot 'packaging/windows/license.rtf'))"/>
  </Package>
</Wix>
"@
$source = Join-Path $OutputFolder "manager-$Architecture.wxs"
[IO.File]::WriteAllText($source, $xml, [Text.UTF8Encoding]::new($false))
$msi = Join-Path $OutputFolder "Craft-Apps-Manager-$version-windows-$Architecture.msi"
if (Test-Path -LiteralPath $msi) { throw "Installer already exists: $msi" }
& $Wix build $source -arch $Architecture -ext WixToolset.UI.wixext -o $msi
if ($LASTEXITCODE -ne 0) { throw 'WiX installer build failed' }
$hash = (Get-FileHash -LiteralPath $msi -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content -LiteralPath "$msi.sha256" -Value "$hash  $(Split-Path -Leaf $msi)" -Encoding ascii
Write-Output $msi
