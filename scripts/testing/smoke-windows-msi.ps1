param(
    [Parameter(Mandatory)][ValidateSet('x64', 'x86')][string]$Architecture,
    [Parameter(Mandatory)][string]$BaselineMsi,
    [Parameter(Mandatory)][string]$CandidateMsi
)
$ErrorActionPreference = 'Stop'
$installer = New-Object -ComObject WindowsInstaller.Installer
function Read-Property([string]$Path, [string]$Name) {
    $db = $installer.OpenDatabase((Resolve-Path -LiteralPath $Path).Path, 0)
    $view = $db.OpenView(('SELECT `Value` FROM `Property` WHERE `Property` = ''{0}''' -f $Name))
    $null = $view.Execute()
    $record = $view.Fetch()
    if (!$record) { throw "Missing MSI property: $Name" }
    $value = $record.StringData(1)
    $null = $view.Close()
    return $value
}
function Run-Msi([string[]]$Arguments) {
    $p = Start-Process msiexec.exe -ArgumentList $Arguments -WindowStyle Hidden -Wait -PassThru
    if ($p.ExitCode -notin @(0, 3010)) { throw "MSI failed with exit code $($p.ExitCode)" }
}
$baselineCode = Read-Property $BaselineMsi 'ProductCode'
$candidateCode = Read-Property $CandidateMsi 'ProductCode'
$version = Read-Property $CandidateMsi 'ProductVersion'
$installFolder = Join-Path $env:LOCALAPPDATA "Programs\Craft Apps Manager $Architecture"
$menu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Craft Apps Manager $Architecture"
if (Test-Path -LiteralPath $installFolder) { throw "Use a clean test machine: $installFolder already exists" }
$logHome = Join-Path ([IO.Path]::GetTempPath()) ("craft-msi-smoke-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $logHome | Out-Null
$cleanupCode = $baselineCode
try {
    Run-Msi @('/i', ('"' + (Resolve-Path $BaselineMsi).Path + '"'), '/qn', '/norestart', '/l*v', ('"' + (Join-Path $logHome 'baseline.log') + '"'))
    Run-Msi @('/i', ('"' + (Resolve-Path $CandidateMsi).Path + '"'), '/qn', '/norestart', '/l*v', ('"' + (Join-Path $logHome 'upgrade.log') + '"'))
    $cleanupCode = $candidateCode
    $exe = Join-Path $installFolder 'CraftApps-Manager.exe'
    if (!(Test-Path -LiteralPath $exe)) { throw 'Upgrade removed the manager executable' }
    $installedVersion = (Get-Item -LiteralPath $exe).VersionInfo.ProductVersion
    if ($installedVersion -ne $version) { throw "Installed executable version '$installedVersion' differs from MSI version '$version'" }
    # The visible software version is unchanged for a hotfix. Verify the actual
    # installed executable revision, rather than only MSI ProductVersion.
    $identityFile = Join-Path $logHome 'installed-identity.json'
    $identityProcess = Start-Process -FilePath $exe -ArgumentList '--build-identity' -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput $identityFile
    if ($identityProcess.ExitCode -ne 0) { throw 'Installed build identity query failed' }
    $identity = Get-Content -LiteralPath $identityFile -Raw | ConvertFrom-Json
    $expectedRevision = if ($env:CRAFT_BUILD_REVISION) { [int]$env:CRAFT_BUILD_REVISION } else { 1 }
    if ($identity.version -ne $version -or $identity.revision -ne $expectedRevision) { throw 'MSI upgrade left an old executable revision installed' }
    foreach ($file in @('installation-msi.json', "tools\7zip\$Architecture\7za.exe")) {
        if (!(Test-Path -LiteralPath (Join-Path $installFolder $file))) { throw "Upgrade removed $file" }
    }
    $shell = New-Object -ComObject WScript.Shell
    foreach ($app in @('Manager', 'Builder')) {
        $path = Join-Path $menu "Craft Apps $app ($Architecture).lnk"
        if (!(Test-Path -LiteralPath $path)) { throw "Upgrade removed the $app shortcut" }
        $shortcut = $shell.CreateShortcut($path)
        if ($shortcut.TargetPath -ne $exe -or !(Test-Path -LiteralPath $shortcut.TargetPath)) { throw "Broken $app shortcut" }
        if ($app -eq 'Builder' -and $shortcut.Arguments -ne '--builder') { throw 'Builder shortcut lost its arguments' }
    }
    Write-Output "Passed $Architecture upgrade to ${version}: executable, tools and both shortcuts survived. Logs: $logHome"
} finally {
    Run-Msi @('/x', $cleanupCode, '/qn', '/norestart')
}
if (Test-Path -LiteralPath (Join-Path $installFolder 'CraftApps-Manager.exe')) { throw 'Uninstall left the manager executable behind' }
