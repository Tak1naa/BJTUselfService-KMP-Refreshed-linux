Name: campus-bjtu
Version: 0.1.10
Release: 1%{?dist}
Summary: 交大自由行 CLI (campus) - Offline-first BJTU campus CLI
License: MIT
Source0: campus-files.tar.gz
BuildArch: x86_64
AutoReqProv: no
# Cargo already stripped this static executable. Keep the exact tested bytes.
%global __strip /bin/true
%description
Linux-native BJTU self-service CLI with an offline SQLite cache.
%prep
%build
%install
mkdir -p %{buildroot}
tar -xzf %{SOURCE0} -C %{buildroot}
%files
/usr/bin/campus
/usr/share/bash-completion/completions/campus
/usr/share/zsh/site-functions/_campus
/usr/share/fish/vendor_completions.d/campus.fish
/usr/share/doc/campus-bjtu/
/usr/share/man/man1/campus.1*

%changelog
* Fri Nov 20 2026 BJTU campus contributors - 0.1.10-1
- Supply GeoTrust G2 CN 2022 intermediate CA hint for *.bjtu.edu.cn when server omits intermediates.

* Thu Nov 19 2026 BJTU campus contributors - 0.1.9-1
- Add PhyVlab support (courses, assignments, details, attachments, draft upload, finalization, loopback captcha login, and TUI tab).

* Thu Nov 19 2026 BJTU campus contributors - 0.1.8-1
- Use the legacy courseware API session ID when the server supplies one.

* Wed Nov 18 2026 BJTU campus contributors - 0.1.7-1
- Clean up homework, show calendar and grade summaries, and align courseware sessions.

* Wed Nov 18 2026 BJTU campus contributors - 0.1.6-1
- Deduplicate matching course meetings and add a weekly schedule view.

* Wed Nov 18 2026 BJTU campus contributors - 0.1.5-1
- Show classroom occupancy as a weekly grid.

* Wed Nov 18 2026 BJTU campus contributors - 0.1.4-1
- Add room selection and courseware downloads to the TUI.

* Wed Nov 18 2026 BJTU campus contributors - 0.1.3-1
- Add opt-in read-only homework compatibility.

* Wed Nov 18 2026 BJTU campus contributors - 0.1.2-1
- Add a fullscreen campus dashboard.

* Wed Nov 18 2026 BJTU campus contributors - 0.1.1-1
- Add styled terminal tables and show sync failures when a cache is missing.

* Wed Nov 18 2026 BJTU campus contributors - 0.1.0-1
- Initial Linux CLI package.
