#!/usr/bin/env python3
"""Create a platform archive containing the CLI and independently installable extensions."""
import argparse
import hashlib
import pathlib
import shutil
import subprocess
import json
import tarfile
import tempfile
import zipfile

parser = argparse.ArgumentParser()
parser.add_argument("target")
parser.add_argument("--version")
args = parser.parse_args()
workspace_version = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"]))["packages"][0]["version"]
if args.version is not None and args.version != workspace_version:
    parser.error("archive version must match workspace version")
args.version = workspace_version
source = pathlib.Path("target") / args.target / "release"
windows = "windows" in args.target
suffix = ".dll" if windows else ".dylib" if "apple" in args.target else ".so"
prefix = "" if windows else "lib"
names = ["syncer.exe" if windows else "syncer"]
names += [f"{prefix}syncer_extension_{name}{suffix}" for name in ["http", "git", "json", "structured"]]
dist = pathlib.Path("dist")
dist.mkdir(exist_ok=True)
archive = dist / f"syncer-{args.version}-{args.target}{'.zip' if windows else '.tar.gz'}"
with tempfile.TemporaryDirectory() as work:
    work = pathlib.Path(work)
    for name in names:
        shutil.copy2(source / name, work / name)
    for name in ["README.md", "LICENSE"]:
        shutil.copy2(name, work / name)
    if windows:
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as output:
            for path in sorted(work.iterdir()):
                output.write(path, path.name)
    else:
        with tarfile.open(archive, "w:gz") as output:
            for path in sorted(work.iterdir()):
                output.add(path, path.name)
checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
archive.with_name(archive.name + ".sha256").write_text(f"{checksum}  {archive.name}\n")
print(archive)
