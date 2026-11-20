#!/usr/bin/env python3
"""Build local DEB/RPM/tar.gz packages; never install them or contact a release server."""
import argparse
import gzip
import json
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument("binary", type=Path)
p.add_argument("--format", choices=["deb", "rpm", "tar", "all"], default="all")
p.add_argument("--output", type=Path, default=Path("artifacts/packages"))
a = p.parse_args()
binary = a.binary.resolve()
out = a.output.resolve()
out.mkdir(parents=True, exist_ok=True)
root = Path(__file__).resolve().parents[1]
version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
if subprocess.check_output([binary, "--version"], text=True).strip() != f"campus {version}":
    raise SystemExit("packaging binary version does not match Cargo.toml")
elf = subprocess.check_output(["readelf", "-l", str(binary)], text=True)
if "INTERP" in elf:
    raise SystemExit("Linux packaging requires the static musl binary")

stage = Path(tempfile.mkdtemp(prefix="campus-package-", dir=out))
rpm_top = out / "rpmbuild"
artifacts = []
try:
    (stage / "usr/bin").mkdir(parents=True)
    shutil.copy2(binary, stage / "usr/bin/campus")
    shells = [
        ("bash", "usr/share/bash-completion/completions/campus", "completions/campus.bash"),
        ("zsh", "usr/share/zsh/site-functions/_campus", "completions/_campus"),
        ("fish", "usr/share/fish/vendor_completions.d/campus.fish", "completions/campus.fish"),
    ]
    for shell, path, _ in shells:
        target = stage / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(subprocess.check_output([binary, "completions", shell]))
    docs = stage / "usr/share/doc/campus-bjtu"
    docs.mkdir(parents=True)
    shutil.copy2(root / "README.md", docs / "README.md")
    shutil.copy2(root.parent / "LICENSE", docs / "copyright")
    man = sorted(
        binary.parent.glob("build/campus-cli-*/out/campus.1"),
        key=lambda p: p.stat().st_mtime_ns,
    )
    if not man:
        man = sorted(
            (root / "target").rglob("campus.1"),
            key=lambda p: p.stat().st_mtime_ns,
        )
    man_gz = stage / "usr/share/man/man1/campus.1.gz"
    if man:
        man_gz.parent.mkdir(parents=True, exist_ok=True)
        man_gz.write_bytes(gzip.compress(man[-1].read_bytes(), mtime=0))

    if a.format in ["deb", "all"]:
        meta = stage / "DEBIAN"
        meta.mkdir()
        (meta / "control").write_text(
            f"Package: campus-bjtu\nVersion: {version}\nArchitecture: amd64\n"
            f"Maintainer: BJTU campus contributors\nSection: utils\nPriority: optional\n"
            f"Description: 交大自由行 CLI (campus) - Offline-first BJTU campus CLI\n"
        )
        path = out / f"campus-bjtu_{version}_amd64.deb"
        subprocess.run(["dpkg-deb", "--root-owner-group", "--build", str(stage), str(path)], check=True)
        artifacts.append(str(path))
        shutil.rmtree(meta)

    if a.format in ["rpm", "all"]:
        for d in ["BUILD", "BUILDROOT", "RPMS", "SOURCES", "SPECS", "SRPMS", "tmp"]:
            (rpm_top / d).mkdir(parents=True, exist_ok=True)
        archive = rpm_top / "SOURCES/campus-files.tar.gz"
        with tarfile.open(archive, "w:gz") as tar:
            for f in sorted(stage.rglob("*")):
                tar.add(f, arcname=f.relative_to(stage), recursive=False)
        spec = (root / "packaging/campus.spec").read_text()
        if not man_gz.exists():
            spec = spec.replace("/usr/share/man/man1/campus.1*\n", "")
        specpath = rpm_top / "SPECS/campus.spec"
        specpath.write_text(spec)
        subprocess.run(
            [
                "rpmbuild",
                "-bb",
                "--define",
                f"_topdir {rpm_top}",
                "--define",
                f"_tmppath {rpm_top / 'tmp'}",
                str(specpath),
            ],
            check=True,
        )
        for path in (rpm_top / "RPMS").rglob("*.rpm"):
            target = out / path.name
            shutil.copy2(path, target)
            artifacts.append(str(target))

    if a.format in ["tar", "all"]:
        tar_path = out / f"campus-bjtu-{version}-linux-x86_64.tar.gz"
        with tarfile.open(tar_path, "w:gz", compresslevel=9) as tar:
            tar.add(stage / "usr/bin/campus", arcname="campus")
            tar.add(docs / "README.md", arcname="README.md")
            tar.add(docs / "copyright", arcname="LICENSE")
            if man_gz.exists():
                tar.add(man_gz, arcname="campus.1.gz")
            for _, stage_rel, tar_rel in shells:
                tar.add(stage / stage_rel, arcname=tar_rel)
        artifacts.append(str(tar_path))
finally:
    shutil.rmtree(stage, ignore_errors=True)
    shutil.rmtree(rpm_top, ignore_errors=True)

print(json.dumps({"packages": artifacts}, indent=2))
