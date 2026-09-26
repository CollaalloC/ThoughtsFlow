#!/usr/bin/env python3
"""Generate release notices from locked, locally fetched sources (Python 3.9+).

Run npm ci and cargo fetch --locked first. --check needs no toolchain or network.
License overrides are reviewed, checked-in source texts, never guessed SPDX text.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "licenses"
LOCKS = ("package-lock.json", "src-tauri/Cargo.lock")
LICENSE_NAME = re.compile(r"^(license|licence|copying|copyright|notice)([._-].*)?$", re.I)
CODE_SUFFIXES = {".rs", ".c", ".h", ".cpp", ".py", ".js", ".ts", ".yml", ".yaml"}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def load(path):
    return json.loads(path.read_text(encoding="utf-8"))


def license_files(directory, recursive=False):
    candidates = directory.rglob("*") if recursive else directory.iterdir()
    return sorted(path for path in candidates if path.is_file() and (
        LICENSE_NAME.match(path.name) or
        any(part.lower() in {"licenses", "licences"} for part in path.relative_to(directory).parts[:-1])
    ) and path.suffix.lower() not in CODE_SUFFIXES)


def text_record(path, source):
    data = path.read_bytes()
    sha = digest(data)
    relative = "texts/" + sha + ".txt"
    (OUT / relative).write_bytes(data)
    return {"file": relative, "sha256": sha, "source": source}


def generate():
    (OUT / "texts").mkdir(parents=True, exist_ok=True)
    (OUT / "mpl-source").mkdir(parents=True, exist_ok=True)
    packages = []
    npm = load(ROOT / "package-lock.json")
    for location, package in sorted(npm["packages"].items()):
        if not location or package.get("dev"):
            continue
        directory = ROOT / location
        manifest = load(directory / "package.json")
        name, version = manifest["name"], package["version"]
        if manifest["version"] != version:
            raise RuntimeError("Run npm ci: installed version differs for " + name)
        files = license_files(directory)
        if not files:
            raise RuntimeError("Missing npm license text: " + name)
        source = "https://registry.npmjs.org/" + name + "/-/" + name.split("/")[-1] + "-" + version + ".tgz"
        packages.append({"ecosystem": "npm", "name": name, "version": version,
                         "license": package.get("license", manifest.get("license")),
                         "source": source, "integrity": package.get("integrity"),
                         "licenseTexts": [text_record(p, source + "#" + p.name) for p in files]})

    cargo = os.environ.get("CARGO") or shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo")
    metadata = json.loads(subprocess.check_output([
        cargo, "metadata", "--manifest-path", str(ROOT / "src-tauri/Cargo.toml"),
        "--format-version", "1", "--locked", "--offline",
    ], cwd=ROOT))
    by_id = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    seen, pending = set(), [metadata["resolve"]["root"]]
    while pending:
        package_id = pending.pop()
        if package_id in seen:
            continue
        seen.add(package_id)
        for dependency in nodes[package_id]["deps"]:
            if any(kind["kind"] != "dev" for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])
    overrides = load(OUT / "overrides.json")
    # Cargo emits these fields as simple quoted strings; parse only its package
    # checksum records so the release checker keeps working on Python 3.9.
    checksums = {}
    for block in (ROOT / "src-tauri/Cargo.lock").read_text(encoding="utf-8").split("[[package]]")[1:]:
        fields = {key: json.loads(value) for key, value in re.findall(
            r'^(name|version|checksum) = ("[^"\n]*")$', block, re.M)}
        if "checksum" in fields:
            checksums[(fields["name"], fields["version"])] = fields["checksum"]
    for package_id in sorted(seen):
        package = by_id[package_id]
        if package_id == metadata["resolve"]["root"]:
            continue
        name, version = package["name"], package["version"]
        directory = Path(package["manifest_path"]).parent
        source = "https://static.crates.io/crates/" + name + "/" + name + "-" + version + ".crate"
        files = license_files(directory, recursive=True)
        texts = [text_record(p, source + "#" + p.relative_to(directory).as_posix()) for p in files]
        for override in overrides.get(name + "@" + version, []):
            record = text_record(OUT / override["file"], override["source"])
            record["note"] = override["note"]
            texts.append(record)
        if not texts or not package["license"]:
            raise RuntimeError("Missing Rust license metadata/text: " + name + "@" + version)
        checksum = checksums[(name, version)]
        record = {"ecosystem": "cargo", "name": name, "version": version,
                  "license": package["license"], "authors": package["authors"],
                  "source": source, "sha256": checksum, "licenseTexts": texts}
        if "MPL-2.0" in package["license"]:
            archive = directory.parent.parent.parent / "cache" / directory.parent.name / (directory.name + ".crate")
            data = archive.read_bytes()
            if digest(data) != checksum:
                raise RuntimeError("Source checksum mismatch: " + name)
            destination = "mpl-source/" + archive.name
            (OUT / destination).write_bytes(data)
            record["sourceArchive"] = destination
        packages.append(record)
    packages.sort(key=lambda p: (p["ecosystem"], p["name"], p["version"]))
    inventory = {"formatVersion": 1,
                 "scope": "npm production dependencies; Cargo normal/build dependency graph across all targets, default features; conservative superset, not a binary SBOM",
                 "lockfiles": {name: digest((ROOT / name).read_bytes()) for name in LOCKS},
                 "packages": packages}
    (OUT / "dependencies.json").write_text(json.dumps(inventory, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    rows = ["# Dependency notices", "", inventory["scope"], "",
            "Original license expressions are retained. Dual-licensed alternatives remain alternatives;",
            "retaining several texts does not combine alternative obligations. Third-party rights are",
            "not replaced by ThoughtsFlow's MIT license. See dependencies.json for integrity and provenance.", "",
            "| Ecosystem | Component | Declared license | License text |", "| --- | --- | --- | --- |"]
    for package in packages:
        links = ", ".join("[" + str(i + 1) + "](" + t["file"] + ")" for i, t in enumerate(package["licenseTexts"]))
        rows.append("| " + package["ecosystem"] + " | [" + package["name"] + " " + package["version"] + "](" + package["source"] + ") | " + package["license"] + " | " + links + " |")
    (OUT / "DEPENDENCIES.md").write_text("\n".join(rows) + "\n", encoding="utf-8")
    print("Generated notices for", sum(p["ecosystem"] == "npm" for p in packages), "npm and",
          sum(p["ecosystem"] == "cargo" for p in packages), "Rust packages.")


def check():
    inventory = load(OUT / "dependencies.json")
    for name in LOCKS:
        if inventory["lockfiles"].get(name) != digest((ROOT / name).read_bytes()):
            raise RuntimeError("Dependency inventory is stale: " + name)
    for package in inventory["packages"]:
        for record in package["licenseTexts"]:
            if digest((OUT / record["file"]).read_bytes()) != record["sha256"]:
                raise RuntimeError("License text changed: " + record["file"])
        if "sourceArchive" in package and digest((OUT / package["sourceArchive"]).read_bytes()) != package["sha256"]:
            raise RuntimeError("Covered source changed: " + package["sourceArchive"])
    print("License inventory matches both lockfiles; all license texts and MPL source hashes verified.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    arguments = parser.parse_args()
    check() if arguments.check else generate()
