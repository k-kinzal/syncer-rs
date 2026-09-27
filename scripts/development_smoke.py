#!/usr/bin/env python3
"""Exercise opt-in adjacent native loading with isolated state and executable."""
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

build = pathlib.Path(sys.argv[1]).resolve()
binary = "syncer.exe" if os.name == "nt" else "syncer"
suffix = ".dll" if os.name == "nt" else ".dylib" if sys.platform == "darwin" else ".so"
prefix = "" if os.name == "nt" else "lib"


def library(name):
    return f"{prefix}syncer_extension_{name}{suffix}"


with tempfile.TemporaryDirectory(prefix="syncer-development-") as temporary:
    root = pathlib.Path(temporary).resolve()
    directory = root / "bin"
    directory.mkdir()
    project = root / "project"
    project.mkdir()
    state = root / "state"
    exe = directory / binary
    shutil.copy2(build / binary, exe)

    def run(*args, dev=None, code=0, executable=exe):
        environment = os.environ.copy()
        environment.pop("SYNCER_DEV_EXTENSIONS", None)
        if dev is not None:
            environment["SYNCER_DEV_EXTENSIONS"] = dev
        result = subprocess.run(
            [str(executable), "--output", "json", "--state-dir", str(state), "--project", str(root), *map(str, args)],
            cwd=project, env=environment, capture_output=True, text=True,
        )
        assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
        return result

    # Current-directory libraries are never searched, and read-only discovery
    # leaves no state. Unknown adjacent library names are not executed either.
    (project / library("json")).write_bytes(b"invalid cwd library")
    (directory / library("unofficial")).write_bytes(b"invalid unrelated library")
    # Older build outputs must not re-enable integrations outside the official set.
    for name in ["claude", "google_drive"]:
        (directory / library(name)).write_bytes(b"stale non-official library")
    assert json.loads(run("extension", "list", dev="1").stdout) == []
    assert not state.exists()

    adjacent = directory / library("json")
    shutil.copy2(build / library("json"), adjacent)
    assert json.loads(run("extension", "list").stdout) == []
    assert json.loads(run("--dev-extensions=false", "extension", "list", dev="1").stdout) == []
    assert json.loads(run("extension", "list", dev="0").stdout) == []
    assert [m["name"] for m in json.loads(run("--dev-extensions", "extension", "list").stdout)] == ["json"]
    assert [m["name"] for m in json.loads(run("extension", "list", dev="1").stdout)] == ["json"]
    assert not state.exists()

    # A symlink to the CLI still discovers its physical siblings (Unix).
    if os.name != "nt":
        alias = project / "syncer-link"
        alias.symlink_to(exe)
        assert len(json.loads(run("extension", "list", dev="1", executable=alias).stdout)) == 1

    # Exercise JSON patches and daemon loading without any extension enrollment.
    policy = root / "policy.hcl"
    policy.write_text('schema_version=1\nname="dev"\nrule "field" {\n target="project/settings.json"\n kind="json"\n operation="set"\n pointer="/managed"\n value=true\n}\n')
    target = project / "settings.json"
    target.write_text('{"personal":"keep"}\n')
    run("add", "dev", policy, dev="1")
    config = (state / "config.json").read_bytes()
    assert json.loads(config)["extensions"] == []
    original = target.read_bytes()
    snapshot = {p.relative_to(state): p.read_bytes() for p in state.rglob("*") if p.is_file()}
    run("apply", "--dry-run", dev="1")
    assert target.read_bytes() == original
    assert snapshot == {p.relative_to(state): p.read_bytes() for p in state.rglob("*") if p.is_file()}
    run("apply", dev="1")
    assert json.loads(target.read_text()) == {"personal": "keep", "managed": True}
    run("apply", "--check", dev="1")
    target.write_bytes(original)
    run("daemon", "--once", dev="1")
    assert json.loads(target.read_text())["managed"] is True
    assert (state / "config.json").read_bytes() == config

    # Explicit pinned installation wins even over a broken adjacent build.
    run("extension", "install", adjacent, dev="1")
    adjacent.write_bytes(b"broken rebuild")
    assert len(json.loads(run("extension", "list", dev="1").stdout)) == 1
    installed = pathlib.Path(json.loads((state / "config.json").read_text())["extensions"][0]["path"])
    pinned = installed.read_bytes()
    installed.write_bytes(pinned + b"tampered")
    assert "digest changed" in run("extension", "list", dev="1", code=1).stderr
    installed.write_bytes(pinned)
    run("extension", "remove", "json", dev="1")
    run("extension", "list", dev="1", code=1)
    shutil.copy2(build / library("json"), adjacent)
    assert len(json.loads(run("extension", "list", dev="1").stdout)) == 1

    # Expected filename alone is insufficient: the manifest name must match.
    mismatch = directory / library("http")
    shutil.copy2(build / library("json"), mismatch)
    assert "expected development extension http" in run("extension", "list", dev="1", code=1).stderr
    mismatch.unlink()
    if os.name != "nt":
        mismatch.symlink_to(adjacent)
        assert "symlink" in run("extension", "list", dev="1", code=1).stderr
        mismatch.unlink()

    # A policy cannot plant a future library in the opted-in build directory,
    # even when no library currently exists there.
    blocked = directory / library("http")
    policy.write_text(f'schema_version=1\nname="dev"\nrule "plant" {{\n target="bin/{blocked.name}"\n kind="file"\n operation="replace"\n value="bad"\n}}\n')
    result = run("apply", dev="1", code=1)
    assert "development extension directory" in result.stderr
    assert not blocked.exists()

    # The actual workspace output includes four independent official extensions.
    for name in ["json", "http", "git", "structured"]:
        shutil.copy2(build / library(name), directory / library(name))
    manifests = json.loads(run("extension", "list", dev="1").stdout)
    assert {m["name"] for m in manifests} == {"json", "http", "git", "structured"}

print("development discovery, precedence, dry-run, apply, daemon and write-protection smoke tests passed")
