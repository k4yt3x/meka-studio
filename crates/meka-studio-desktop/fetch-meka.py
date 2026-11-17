#!/usr/bin/env python3
"""Fetches the Meka the desktop app is built with: the release meka.json pins, for one target,
checked against its SHA-256, into binaries/ beside this script."""

import argparse
import hashlib
import io
import json
import pathlib
import sys
import tarfile
import urllib.request
import zipfile

HERE = pathlib.Path(__file__).resolve().parent

# The most of a release asset that is read.
MAX_BYTES = 256 * 1024 * 1024


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", help="the Rust target, such as x86_64-unknown-linux-gnu")
    target = parser.parse_args().target
    pin = json.loads((HERE / "meka.json").read_text())
    versions = json.loads((HERE.parents[1] / "web/src/api/meka-versions.json").read_text())
    if pin["version"] not in versions["verified"]:
        sys.exit(
            f"Meka {pin['version']} is not among the versions Meka Studio was checked against "
            f"({', '.join(versions['verified'])}) in web/src/api/meka-versions.json"
        )
    asset = pin["assets"].get(target)
    if asset is None:
        sys.exit(f"Meka has no release for {target}; it has {', '.join(pin['assets'])}")
    url = pin["url"].format(version=pin["version"], file=asset["file"])
    with urllib.request.urlopen(url, timeout=300) as response:
        data = response.read(MAX_BYTES + 1)
    if len(data) > MAX_BYTES:
        sys.exit(f"{url} is larger than {MAX_BYTES} bytes")
    digest = hashlib.sha256(data).hexdigest()
    if digest != asset["sha256"]:
        sys.exit(f"{url} has the SHA-256 {digest}, not the {asset['sha256']} meka.json pins")
    name = "meka.exe" if "windows" in target else "meka"
    binary = None
    if asset["file"].endswith(".zip"):
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            member = next(
                (m for m in archive.namelist() if pathlib.PurePosixPath(m).name == name), None
            )
            if member is not None:
                binary = archive.read(member)
    else:
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
            member = next(
                (
                    m
                    for m in archive.getmembers()
                    if m.isfile() and pathlib.PurePosixPath(m.name).name == name
                ),
                None,
            )
            if member is not None:
                binary = archive.extractfile(member).read()
    if binary is None:
        sys.exit(f"{url} holds no {name}")
    out = HERE / "binaries" / name
    out.parent.mkdir(exist_ok=True)
    out.write_bytes(binary)
    out.chmod(0o755)
    print(f"Meka {pin['version']} for {target}: {out}")


if __name__ == "__main__":
    main()
