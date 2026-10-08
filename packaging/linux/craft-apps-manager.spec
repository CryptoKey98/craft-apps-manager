%global debug_package %{nil}
Name:           craft-apps-manager
Version:        %{manager_version}
Release:        0.2%{?dist}
Summary:        Manage Craft app releases, sources and builds
License:        MIT
URL:            https://github.com/CryptoKey98/craft-apps-manager
Source0:        package.tar.gz
Requires:       libX11%{?_isa}, libXrandr%{?_isa}, libXi%{?_isa}, libXcursor%{?_isa}, libxkbcommon%{?_isa}, libwayland-client%{?_isa}, libwayland-cursor%{?_isa}, libwayland-egl%{?_isa}, mesa-libEGL%{?_isa}, mesa-libGL%{?_isa}
Requires:       /usr/bin/notify-send, /usr/bin/xdg-open, /usr/bin/pkexec
Recommends:     7zip

%description
Craft Apps Manager provides a desktop interface for installing Craft apps,
checking for new releases, managing source archives and building executables.
Each user keeps their settings and library in their own data directory.

%prep
%setup -q -c

%build
# A native release binary is staged by scripts/package-linux-rpm.sh.

%install
mkdir -p %{buildroot}%{_bindir} %{buildroot}%{_datadir}/applications %{buildroot}%{_datadir}/pixmaps
install -m 0755 craft-apps-manager %{buildroot}%{_bindir}/craft-apps-manager
install -m 0644 craft-apps-manager.desktop craft-apps-builder.desktop %{buildroot}%{_datadir}/applications/
install -m 0644 icon.png %{buildroot}%{_datadir}/pixmaps/craft-apps-manager.png

%files
%license LICENSE THIRD-PARTY-NOTICES.txt licenses/
%doc linux.md
%{_bindir}/craft-apps-manager
%{_datadir}/applications/craft-apps-manager.desktop
%{_datadir}/applications/craft-apps-builder.desktop
%{_datadir}/pixmaps/craft-apps-manager.png
