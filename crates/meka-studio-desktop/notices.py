#!/usr/bin/env python3
"""Writes the third-party notices the desktop app ships with: docs/third-party.md, then Meka,
then the license texts of the Rust crates built into the app for one target and of the npm
packages built into its interface, each text once with the packages that carry it. It reads the
crates from `cargo metadata` and the packages from web/package-lock.json and web/node_modules,
so `npm ci` must have run in web/."""

import argparse
import json
import pathlib
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]

# Files in a package's folder that hold its license or notices.
LICENSE_FILE = re.compile(r"^(licen[cs]e|copying|notice|unlicense|copyright)([-._].*)?$", re.I)

# Development packages whose files the interface's build takes in: Material Icon Theme's icons,
# Tailwind's base styles, Vite's module preloading and Rolldown's module runtime.
BUNDLED_FROM_DEV = {"material-icon-theme", "tailwindcss", "vite", "rolldown"}

# Dependencies the interface's build never takes in: the canvas PDF.js draws with under Node, one
# package for each system.
NOT_BUNDLED = re.compile(r"^@napi-rs/canvas(-.+)?$")

# License files kept below a package's top folder, beside files of it the interface's build takes
# in: PDF.js's character maps, fonts, and image decoders built to JavaScript.
NESTED_LICENSES = {
    "pdfjs-dist": [
        "cmaps/LICENSE",
        "standard_fonts/LICENSE_FOXIT",
        "standard_fonts/LICENSE_LIBERATION",
        "wasm/LICENSE_JBIG2",
        "wasm/LICENSE_PDFJS_JBIG2",
        "wasm/LICENSE_OPENJPEG",
        "wasm/LICENSE_PDFJS_OPENJPEG",
    ],
}


def license_texts(folder: pathlib.Path, named: str | None) -> list[str]:
    """The license texts in a package's `folder`, with the file its manifest `named` first."""
    files = sorted(
        path for path in folder.iterdir() if path.is_file() and LICENSE_FILE.match(path.name)
    )
    if named:
        path = (folder / named).resolve()
        if path.is_file():
            files = [path] + [file for file in files if file.resolve() != path]
    return [text for text in (read(file) for file in files) if text]


def read(path: pathlib.Path) -> str:
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    return "\n".join(line.rstrip() for line in lines).strip()


def rust_packages(target: str) -> list[tuple[str, str, list[str]]]:
    """The crates built into the app for `target`: name and version, license, license texts."""
    metadata = json.loads(
        subprocess.run(
            [
                "cargo",
                "metadata",
                "--format-version",
                "1",
                "--locked",
                "--filter-platform",
                target,
                "--manifest-path",
                str(ROOT / "Cargo.toml"),
            ],
            check=True,
            capture_output=True,
            encoding="utf-8",
        ).stdout
    )
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    members = set(metadata["workspace_members"])
    app = next(
        package_id
        for package_id in members
        if packages[package_id]["name"] == "meka-studio-desktop"
    )
    # Normal dependencies only: build dependencies and tests are not part of the app.
    seen, queue = set(), [app]
    while queue:
        package_id = queue.pop()
        if package_id in seen:
            continue
        seen.add(package_id)
        for dep in nodes[package_id]["deps"]:
            if any(kind["kind"] is None for kind in dep["dep_kinds"]):
                queue.append(dep["pkg"])
    found = []
    for package in sorted(
        (packages[package_id] for package_id in seen - members),
        key=lambda package: (package["name"], package["version"]),
    ):
        folder = pathlib.Path(package["manifest_path"]).parent
        found.append(
            (
                f"{package['name']} {package['version']}",
                package.get("license") or "no license stated",
                license_texts(folder, package.get("license_file")),
            )
        )
    return found


def npm_packages() -> list[tuple[str, str, list[str]]]:
    """The npm packages built into the interface: name and version, license, license texts."""
    web = ROOT / "web"
    if not (web / "node_modules").is_dir():
        sys.exit("web/node_modules is missing: run npm ci in web/ first")
    lock = json.loads((web / "package-lock.json").read_text(encoding="utf-8"))
    found = []
    for path, package in sorted(lock["packages"].items()):
        if "node_modules/" not in path or package.get("link"):
            continue
        name = path.rsplit("node_modules/", 1)[1]
        development = package.get("dev") or package.get("devOptional")
        if (development and name not in BUNDLED_FROM_DEV) or NOT_BUNDLED.match(name):
            continue
        folder = web / path
        if not folder.is_dir():
            # An optional package for another system, which npm did not install here.
            continue
        nested = [read(folder / file) for file in NESTED_LICENSES.get(name, [])]
        found.append(
            (
                f"{name} {package.get('version', '')}".strip(),
                package.get("license") or "no license stated",
                license_texts(folder, None) + nested,
            )
        )
    return found


def section(title: str, packages: list[tuple[str, str, list[str]]]) -> str:
    """`packages` under `title`, each license text once, after the packages that carry it."""
    texts: dict[str, list[str]] = {}
    without = []
    for name, license, package_texts in packages:
        if not package_texts:
            without.append(f"  {name}: {license}")
        for text in dict.fromkeys(package_texts):
            texts.setdefault(text, []).append(name)
    parts = [f"{'=' * 100}\n{title}\n{'=' * 100}"]
    for text, names in texts.items():
        parts.append(f"{'-' * 100}\nUsed by: {', '.join(names)}\n{'-' * 100}\n\n{text}")
    if without:
        parts.append(
            f"{'-' * 100}\nThese carry no license file, and are under the license they state:\n"
            + "\n".join(without)
        )
    return "\n\n".join(parts)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", help="the Rust target, such as x86_64-pc-windows-msvc")
    parser.add_argument("output", type=pathlib.Path, help="the file to write")
    arguments = parser.parse_args()
    meka = json.loads((HERE / "meka.json").read_text(encoding="utf-8"))["version"]
    parts = [
        read(ROOT / "docs" / "third-party.md"),
        f"{'=' * 100}\nMeka {meka}\n{'=' * 100}\n\n"
        "The desktop app runs Meka, which is licensed under AGPL-3.0-or-later, as Meka Studio is; "
        "LICENSE holds its text. Its source, with what it is built from, is at "
        f"https://github.com/k4yt3x/meka/tree/{meka}.",
        section("Rust crates", rust_packages(arguments.target)),
        section("npm packages, in the interface", npm_packages()),
    ]
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text("\n\n\n".join(parts) + "\n", encoding="utf-8")
    print(f"Third-party notices for {arguments.target}: {arguments.output}")


if __name__ == "__main__":
    main()
