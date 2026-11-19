#!/usr/bin/env python3
"""Package a Windows x64 PE executable and its optional PowerShell launcher."""

import argparse
import struct
import tomllib
import zipfile
from pathlib import Path

MAX_BYTES = 20_000_000
ROOT = Path(__file__).resolve().parents[1]


def check_exe(path: Path) -> None:
    if path.stat().st_size > MAX_BYTES:
        raise SystemExit("campus.exe exceeds the package size budget")
    with path.open("rb") as binary:
        header = binary.read(64)
        if len(header) != 64 or header[:2] != b"MZ":
            raise SystemExit("expected a Windows PE executable")
        binary.seek(struct.unpack_from("<I", header, 60)[0])
        if binary.read(6) != b"PE\0\0\x64\x86":
            raise SystemExit("expected a Windows x64 PE executable")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--output", type=Path, default=ROOT / "artifacts" / "packages")
    args = parser.parse_args()
    check_exe(args.binary)
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    args.output.mkdir(parents=True, exist_ok=True)
    archive = args.output / f"campus-bjtu-{version}-windows-x64.zip"
    windows = ROOT / "packaging" / "windows"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as package:
        package.write(args.binary, "campus.exe")
        package.write(windows / "campus.ps1", "campus.ps1")
        package.write(windows / "README.txt", "README.txt")
        package.write(ROOT.parent / "LICENSE", "LICENSE")
    if archive.stat().st_size > MAX_BYTES:
        archive.unlink()
        raise SystemExit("ZIP exceeds 20 MB")
    with zipfile.ZipFile(archive) as package:
        if package.testzip() is not None:
            raise SystemExit("invalid ZIP")
    print(archive)


if __name__ == "__main__":
    main()
