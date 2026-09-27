"""Packaging must ignore stale and locally built libraries in the build directory."""
import contextlib
import io
import json
import os
from pathlib import Path
import runpy
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile


SCRIPT = Path(__file__).with_name("package.py").resolve()


@contextlib.contextmanager
def working_directory(path):
    previous = Path.cwd()
    os.chdir(path)
    try:
        yield
    finally:
        os.chdir(previous)


class PackageTests(unittest.TestCase):
    def test_archives_exclude_stale_and_unofficial_libraries(self):
        for target, prefix, suffix, binary in [
            ("aarch64-apple-darwin", "lib", ".dylib", "syncer"),
            ("x86_64-unknown-linux-gnu", "lib", ".so", "syncer"),
            ("x86_64-pc-windows-msvc", "", ".dll", "syncer.exe"),
        ]:
            with self.subTest(target=target), tempfile.TemporaryDirectory() as directory:
                root = Path(directory).resolve()
                build = root / "target" / target / "release"
                build.mkdir(parents=True)
                expected = {binary, "README.md", "LICENSE"}
                for name in ["http", "git", "json", "structured"]:
                    expected.add(f"{prefix}syncer_extension_{name}{suffix}")
                for name in expected - {"README.md", "LICENSE"}:
                    (build / name).write_bytes(b"official artifact")
                for name in ["claude", "google_drive", "unofficial"]:
                    (build / f"{prefix}syncer_extension_{name}{suffix}").write_bytes(b"excluded artifact")
                for name in ["README.md", "LICENSE"]:
                    (root / name).write_text(name)
                with (
                    working_directory(root),
                    patch("sys.argv", [str(SCRIPT), target]),
                    patch("subprocess.check_output", return_value=json.dumps({"packages": [{"version": "0.2.0"}]}).encode()),
                    contextlib.redirect_stdout(io.StringIO()),
                ):
                    runpy.run_path(str(SCRIPT), run_name="__main__")
                extension = ".zip" if suffix == ".dll" else ".tar.gz"
                archive = root / "dist" / f"syncer-0.2.0-{target}{extension}"
                if suffix == ".dll":
                    with zipfile.ZipFile(archive) as contents:
                        names = set(contents.namelist())
                else:
                    with tarfile.open(archive) as contents:
                        names = set(contents.getnames())
                self.assertEqual(names, expected)


if __name__ == "__main__":
    unittest.main()
