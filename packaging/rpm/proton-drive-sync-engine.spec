# RPM spec for Fedora / COPR (P3, osirison/proton-drive-sync-engine#95).
#
# Packages the whole desktop app: the daemon + control CLI (built from the root crate) and the
# Tauri desktop GUI (workspace member gui/src-tauri), plus a packaged systemd --user unit,
# freedesktop launcher/icon/AppStream assets (P2, packaging/freedesktop/), and the S10 file-manager
# emblem assets (packaging/emblems/) as separate optional subpackages.
#
# Build strategy: this workspace pulls in the Tauri/webkit2gtk Rust binding stack, which is not
# individually packaged in Fedora's crate ecosystem, so this spec does NOT follow the strict
# Fedora "one subpackage per crate" (cargo2rpm) guidelines used for official Fedora Rust
# packages — that would require hundreds of crate subpackages that mostly don't exist upstream.
# Instead it does a plain, network-fetching `cargo build --release --locked` against crates.io,
# which is the common, pragmatic pattern for large Rust GUI apps distributed via COPR. This means
# the COPR project (or local mock invocation) MUST allow network access during the build step —
# see packaging/rpm/README.md for the exact commands.
#
# Runtime linkage to GTK3/WebKitGTK/libsoup3/dbus/cairo/glib is picked up automatically by RPM's
# ELF/soname dependency generator once the binaries are actually built (it scans the buildroot
# during install), so this spec intentionally does NOT hand-list `Requires: gtk3` etc. — see
# https://docs.fedoraproject.org/en-US/packaging-guidelines/#_shared_libraries (manual Requires
# for auto-detected shared libraries trips rpmlint's explicit-lib-dependency check and tends to
# drift from the real linkage). Confirmed against a real build in this environment: `rpm -qp
# --requires` on the built binary RPM lists exactly libc/libgcc_s/libm/libcairo/libdbus-1/libgdk-3/
# libgdk_pixbuf-2.0/libgio-2.0/libglib-2.0/libgobject-2.0/libgtk-3/libjavascriptcoregtk-4.1/
# libsoup-3.0/libwebkit2gtk-4.1, auto-generated, no manual entries needed. Non-linked runtime
# dependencies (shelled-out binaries, theme/data packages, systemd, dlopen'd libs) are still
# listed explicitly below.
#
# No debuginfo: the workspace's [profile.release] does not enable debug info (Cargo's default),
# so find-debuginfo would produce an empty debugsourcefiles.list and fail the build. Disable the
# debuginfo subpackage rather than fight that here; producing real Rust debuginfo (RUSTFLAGS
# -Cdebuginfo=2 + a working debugsource layout) is a reasonable follow-up, not done in this pass.
%global debug_package %{nil}

# The git tag this Source0 expects to exist at release time. GitHub's source archives keep the
# *tag* as the top-level directory name (repo-name + tag, verbatim) regardless of the filename
# requested in the URL, so the prep stage unpacks from name-rpm_tag, not name-version.
%global rpm_tag v%{version}

# Off by default: the workspace's unit/integration test suite needs network (crates.io was
# already fetched during the build stage) and several minutes, which isn't appropriate for
# every COPR build.
# Opt in with `rpmbuild --with tests` / `mock ... --with tests`.
%bcond_with tests

Name:           proton-drive-sync-engine
Version:        0.1.0
Release:        1%{?dist}
Summary:        Two-way file sync for Proton Drive

License:        Apache-2.0
URL:            https://github.com/osirison/proton-drive-sync-engine
# No v0.1.0 tag exists yet at the time this spec was written (see packaging/rpm/README.md for how
# to build from the working tree / a Copr SCM webhook in the meantime).
Source0:        %{url}/archive/refs/tags/%{rpm_tag}/%{name}-%{version}.tar.gz
# Packaged copy of examples/systemd/proton-syncd.service with ExecStart repointed at
# /usr/bin/proton-syncd for a distro install (see comment in the file itself).
Source1:        proton-syncd.service

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gcc
BuildRequires:  binutils
BuildRequires:  pkgconfig
BuildRequires:  webkit2gtk4.1-devel
BuildRequires:  libsoup3-devel
BuildRequires:  gtk3-devel
BuildRequires:  glib2-devel
BuildRequires:  dbus-devel
BuildRequires:  systemd-rpm-macros

# Non-linked runtime dependencies (not picked up by the auto soname dependency generator):
#  - curl:        src/session.rs shells out to `curl` for the volume-events HTTP transport.
#  - libsecret:   src/session.rs shells out to `secret-tool` (from libsecret) to read the
#                 proton-drive CLI's cached session out of the desktop keyring.
#  - hicolor-icon-theme: owns the /usr/share/icons/hicolor tree this package installs icons into.
Requires:       curl
Requires:       libsecret
Requires:       hicolor-icon-theme
# Soft dependency, not BuildRequires/Requires: tray-icon's Linux backend (pulled in via Tauri's
# "tray-icon" feature) loads libappindicator3/libayatana-appindicator3 with dlopen at runtime
# (libappindicator-sys uses `libloading`, no headers/pkg-config/link-time dependency at all —
# verified against a real build in this environment: the built proton-sync-gui binary has no
# libappindicator3 in its ELF NEEDED list or in RPM's auto-generated Requires). Recommend it so
# the system tray icon works out of the box, without hard-requiring it.
Recommends:     libappindicator-gtk3
%{?systemd_requires}

%description
Proton Drive Sync is a two-way file-synchronization engine between a local
folder and Proton Drive. This package provides the long-running sync daemon
(proton-syncd), its control CLI (proton-sync), and the Tauri desktop GUI
(proton-sync-gui), together with a systemd user unit and Linux desktop
integration (application launcher, hicolor icons, AppStream metadata).

IMPORTANT — external dependency: proton-syncd shells out to the official
`proton-drive` CLI to perform Proton Drive API operations, and reads that
CLI's logged-in session from the desktop keyring via `secret-tool`. The
`proton-drive` CLI is a separate, user-provided binary that is NOT packaged
here (it is not available in Fedora/COPR repositories); install it yourself
and make sure it is on $PATH before starting proton-syncd. `secret-tool`
(from libsecret) IS pulled in as a package dependency.

After installing, enable the daemon per-user with:
    systemctl --user enable --now proton-syncd

%package nautilus
Summary:        Nautilus file-manager sync-status emblems for %{name}
BuildArch:      noarch
Requires:       %{name}%{?_isa} = %{version}-%{release}
Requires:       nautilus-python

%description nautilus
GNOME Files (Nautilus) extension that shows a per-file emblem
(synced / syncing / conflict) by reading the sync engine's SQLite index
read-only. Split out from the main package so headless users don't have to
pull in nautilus-python/GObject introspection. Restart Nautilus after
installing (`nautilus -q`) to load the extension.

%package nemo
Summary:        Nemo file-manager sync-status emblems for %{name}
BuildArch:      noarch
Requires:       %{name}%{?_isa} = %{version}-%{release}
Requires:       nemo-python

%description nemo
Nemo (Cinnamon Files) extension that shows a per-file emblem
(synced / syncing / conflict) by reading the sync engine's SQLite index
read-only. Split out from the main package so headless users don't have to
pull in nemo-python/GObject introspection. Restart Nemo after installing
(`nemo -q`) to load the extension.

%prep
%autosetup -n %{name}-%{rpm_tag}

%build
# Root crate: the daemon + control CLI.
cargo build --release --bins --locked
# Workspace member gui/src-tauri: the desktop GUI (package name proton-sync-gui, has
# default-run). Needs the webkit2gtk4.1/libsoup3/gtk3/glib2 -devel BuildRequires above to link.
cargo build --release -p proton-sync-gui --locked

%install
install -Dm0755 target/release/proton-syncd %{buildroot}%{_bindir}/proton-syncd
install -Dm0755 target/release/proton-sync %{buildroot}%{_bindir}/proton-sync
install -Dm0755 target/release/proton-sync-gui %{buildroot}%{_bindir}/proton-sync-gui

# The debug_package override above disables RPM's automatic strip pass, so strip the release
# binaries explicitly — otherwise they ship unstripped (rpmlint: unstripped-binary-or-object).
strip %{buildroot}%{_bindir}/proton-syncd \
      %{buildroot}%{_bindir}/proton-sync \
      %{buildroot}%{_bindir}/proton-sync-gui

# systemd user unit (packaged copy, see Source1 comment above).
install -Dm0644 %{SOURCE1} %{buildroot}%{_userunitdir}/proton-syncd.service

# Freedesktop launcher / AppStream / icons (P2, packaging/freedesktop/).
install -Dm0644 packaging/freedesktop/app.protondrivesync.engine.desktop \
    %{buildroot}%{_datadir}/applications/app.protondrivesync.engine.desktop
install -Dm0644 packaging/freedesktop/app.protondrivesync.engine.metainfo.xml \
    %{buildroot}%{_datadir}/metainfo/app.protondrivesync.engine.metainfo.xml

# Install whatever hicolor apps-icon sizes actually exist in the source tree, rather than
# hardcoding a size list that can drift from packaging/freedesktop/icons/hicolor/*.
for size_dir in packaging/freedesktop/icons/hicolor/*x*/apps; do
    # An unmatched glob is left literal in POSIX sh; skip it so the loop is a real no-op
    # (not a failed install) if no sized apps-icon dirs exist.
    [ -d "${size_dir}" ] || continue
    size=$(basename "$(dirname "${size_dir}")")
    install -Dm0644 "${size_dir}/app.protondrivesync.engine.png" \
        "%{buildroot}%{_datadir}/icons/hicolor/${size}/apps/app.protondrivesync.engine.png"
done
install -Dm0644 packaging/freedesktop/icons/hicolor/scalable/apps/app.protondrivesync.engine.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/app.protondrivesync.engine.svg

# S10 emblem icons ship in the main package (packaging/emblems/); the extensions that use them
# are split into the -nautilus / -nemo subpackages below.
install -d %{buildroot}%{_datadir}/icons/hicolor/scalable/emblems
for f in packaging/emblems/icons/hicolor/scalable/emblems/emblem-proton-sync-*.svg; do
    [ -e "${f}" ] || continue  # skip the literal glob if no emblem SVGs are present
    install -m0644 "${f}" %{buildroot}%{_datadir}/icons/hicolor/scalable/emblems/
done

install -Dm0644 packaging/emblems/nautilus/proton-sync-nautilus.py \
    %{buildroot}%{_datadir}/nautilus-python/extensions/proton-sync-nautilus.py
install -Dm0644 packaging/emblems/nemo/proton-sync-nemo.py \
    %{buildroot}%{_datadir}/nemo-python/extensions/proton-sync-nemo.py

%if %{with tests}
%check
cargo test --release --all-targets --all-features --locked
%endif

%files
%license LICENSE
%doc README.md examples/proton-sync.toml
%{_bindir}/proton-syncd
%{_bindir}/proton-sync
%{_bindir}/proton-sync-gui
%{_userunitdir}/proton-syncd.service
%{_datadir}/applications/app.protondrivesync.engine.desktop
%{_datadir}/metainfo/app.protondrivesync.engine.metainfo.xml
%{_datadir}/icons/hicolor/*/apps/app.protondrivesync.engine.png
%{_datadir}/icons/hicolor/scalable/apps/app.protondrivesync.engine.svg
%{_datadir}/icons/hicolor/scalable/emblems/emblem-proton-sync-*.svg

%files nautilus
%{_datadir}/nautilus-python/extensions/proton-sync-nautilus.py

%files nemo
%{_datadir}/nemo-python/extensions/proton-sync-nemo.py

%post
if [ -x %{_bindir}/gtk-update-icon-cache ]; then
    %{_bindir}/gtk-update-icon-cache -q -t -f %{_datadir}/icons/hicolor >/dev/null 2>&1 || :
fi
if [ -x %{_bindir}/update-desktop-database ]; then
    %{_bindir}/update-desktop-database -q %{_datadir}/applications >/dev/null 2>&1 || :
fi
if [ -x %{_bindir}/appstreamcli ]; then
    %{_bindir}/appstreamcli refresh --force >/dev/null 2>&1 || :
fi
%systemd_user_post proton-syncd.service

%preun
%systemd_user_preun proton-syncd.service

%postun
if [ -x %{_bindir}/gtk-update-icon-cache ]; then
    %{_bindir}/gtk-update-icon-cache -q -t -f %{_datadir}/icons/hicolor >/dev/null 2>&1 || :
fi
if [ -x %{_bindir}/update-desktop-database ]; then
    %{_bindir}/update-desktop-database -q %{_datadir}/applications >/dev/null 2>&1 || :
fi
if [ -x %{_bindir}/appstreamcli ]; then
    %{_bindir}/appstreamcli refresh --force >/dev/null 2>&1 || :
fi
%systemd_user_postun_with_restart proton-syncd.service

%changelog
* Sun Jul 26 2026 Mina <mina.swe@gmail.com> - 0.1.0-1
- Initial RPM packaging for Fedora/COPR (#95): daemon, control CLI, Tauri GUI, systemd user
  unit, freedesktop launcher/icon/AppStream assets, and nautilus/nemo emblem subpackages.
