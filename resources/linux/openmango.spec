# Binary RPM from the tree scripts/release_rpm.sh stages: nothing is compiled here.
# Bun embeds its payload in the mongosh-sidecar ELF file; stripping or rewriting it breaks it.
%global __os_install_post %{nil}
%global debug_package %{nil}
%global _build_id_links none

Name:           openmango
Version:        %{openmango_version}
Release:        1%{?dist}
Summary:        Browse, query, and manage MongoDB databases
License:        GPL-3.0-or-later
URL:            https://github.com/ggagosh/openmango
ExclusiveArch:  x86_64 aarch64
# The bundled MongoDB tools are private to OpenMango; don't advertise them to other packages.
AutoProv:       no
Requires:       hicolor-icon-theme

%description
OpenMango is a native desktop client to browse, query, and manage MongoDB databases.

%install
cp -a %{openmango_root}/. %{buildroot}/

%files
%{_bindir}/openmango
%{_prefix}/lib/openmango
%{_datadir}/applications/com.openmango.app.desktop
%{_datadir}/icons/hicolor/256x256/apps/com.openmango.app.png
%doc %{_docdir}/openmango
