# Windows packages

The manager is available as a portable ZIP or a per-user MSI, separately for x64 and x86. The MSI does not require a Rust toolchain, .NET, or a separate 7-Zip installation on the user's computer.

The MSI installs under `%LOCALAPPDATA%\Programs\Craft Apps Manager x64` (or `x86`) and adds Manager and Builder Start menu shortcuts. Windows Installed apps handles removal. The app library, settings, backups, sources, and build history live in `%LOCALAPPDATA%\Craft Apps Manager`, outside the MSI-owned directory. Uninstall keeps that data. Both MSI architectures share this library, so avoid simultaneous operations.

Portable builds keep their library next to the executable. Moving from portable to MSI does not delete or automatically copy the portable library; select the existing library folder in Settings if you want to keep using it.

## Building

Build the Rust release for `x86_64-pc-windows-msvc` or `i686-pc-windows-msvc`. Packaging requires PowerShell, a matching portable 7-Zip SDK folder with license notices, and WiX 4.0.6 with its UI extension. WiX needs the .NET SDK on the packaging computer only:

```powershell
dotnet tool install --global wix --version 4.0.6
wix extension add -g WixToolset.UI.wixext/4.0.6
./scripts/package-windows-msi.ps1 -Architecture x64 -SevenZipFolder C:/tools/7zip/x64
```

Use `-TargetFolder` for an external Cargo target directory and `-OutputFolder` for external package output. The package script rejects the wrong executable or 7-Zip architecture. It creates an MSI and a SHA-256 checksum file. The portable packaging script remains available separately.

Upgrade codes are stable and distinct for x86 and x64. Each new version receives a new MSI product code. Windows Installer rejects downgrades and upgrades the matching architecture in place. Keep those upgrade codes unchanged in future releases.

## Manager updates

The MSI owns `installation-msi.json` beside the executable. Installed copies request the matching architecture's MSI release asset; portable copies request the ZIP. The downloaded package is checked by the normal release download verifier. After the manager closes, its copied Rust helper waits for the process to exit, runs Windows Installer, and restarts the manager with the original data and tool locations. Installer errors and cancellation leave the previous installation under Windows Installer's control, with a diagnostic log in the library's `runtime/self-update` folder.

Do not copy the MSI marker into a portable package. The manager never replaces MSI-owned files through the portable ZIP updater.
