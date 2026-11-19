#!/usr/bin/env python3
"""Package a macOS Mach-O / Universal binary into a .tar.gz release archive."""

import argparse
import io
import os
import subprocess
import tarfile
import time
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MACHO_MAGICS = {
    b"\xfe\xed\xfa\xcf",  # MH_MAGIC_64 (big-endian)
    b"\xcf\xfa\xed\xfe",  # MH_MAGIC_64 (little-endian)
    b"\xca\xfe\xba\xbe",  # FAT_MAGIC (universal)
    b"\xbe\xba\xfe\xca",  # FAT_CIGAM (universal)
}
MAX_BYTES = {
    "arm64": 15_000_000,
    "x64": 15_000_000,
    "universal": 25_000_000,
}


def check_macho(path: Path, arch: str) -> None:
    size = path.stat().st_size
    if size > MAX_BYTES[arch]:
        raise SystemExit(f"campus exceeds {arch} size budget: {size} bytes")
    with path.open("rb") as binary:
        magic = binary.read(4)
    if magic not in MACHO_MAGICS:
        raise SystemExit(f"expected a 64-bit Mach-O or Universal binary, got magic {magic!r}")


def add_bytes(tar: tarfile.TarFile, arcname: str, data: bytes, mode: int) -> None:
    info = tarfile.TarInfo(name=arcname)
    info.size = len(data)
    info.mode = mode
    info.mtime = int(time.time())
    tar.addfile(info, io.BytesIO(data))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--arch", choices=["universal", "arm64", "x64"], default="universal")
    parser.add_argument("--output", type=Path, default=ROOT / "artifacts" / "packages")
    args = parser.parse_args()

    binary = args.binary.resolve()
    check_macho(binary, args.arch)
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if os.uname().sysname == "Darwin":
        actual = subprocess.check_output([binary, "--version"], text=True).strip()
        if actual != f"campus {version}":
            raise SystemExit(f"binary version {actual!r} does not match Cargo.toml {version!r}")

    args.output.mkdir(parents=True, exist_ok=True)
    archive = args.output / f"campus-bjtu-{version}-macos-{args.arch}.tar.gz"
    macos = ROOT / "packaging" / "macos"

    with tarfile.open(archive, "w:gz", compresslevel=9) as package:
        add_bytes(package, "campus", binary.read_bytes(), 0o755)
        add_bytes(package, "README.txt", (macos / "README.txt").read_bytes(), 0o644)
        add_bytes(package, "LICENSE", (ROOT.parent / "LICENSE").read_bytes(), 0o644)
        if os.uname().sysname == "Darwin":
            for shell, filename in [
                ("bash", "completions/campus.bash"),
                ("zsh", "completions/_campus"),
                ("fish", "completions/campus.fish"),
            ]:
                comp = subprocess.check_output([binary, "completions", shell])
                add_bytes(package, filename, comp, 0o644)

    with tarfile.open(archive, "r:gz") as package:
        members = package.getnames()
        if "campus" not in members:
            raise SystemExit("archive missing campus binary")
    print(archive)


if __name__ == "__main__":
    main()
