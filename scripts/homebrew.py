#!/usr/bin/env python3
"""Generate a binary Homebrew formula from verified local release archives."""
import hashlib
import pathlib
import sys
version, directory = sys.argv[1], pathlib.Path(sys.argv[2])
print('''class Syncer < Formula
  desc "Layered, patch-first file policy synchronization"
  homepage "https://github.com/k-kinzal/syncer-rs"
  license "MIT"''')
print(f'  version "{version}"')
for os_name, targets in [("macos", [("arm", "aarch64-apple-darwin"), ("intel", "x86_64-apple-darwin")]), ("linux", [("arm", "aarch64-unknown-linux-gnu"), ("intel", "x86_64-unknown-linux-gnu")])]:
    print(f"  on_{os_name} do")
    for arch, target in targets:
        name = f"syncer-{version}-{target}.tar.gz"
        digest = hashlib.sha256((directory / name).read_bytes()).hexdigest()
        print(f'''    on_{arch} do
      url "https://github.com/k-kinzal/syncer-rs/releases/download/v{version}/{name}"
      sha256 "{digest}"
    end''')
    print("  end")
print('''
  def install
    bin.install "syncer"
    (lib/"syncer").install Dir["libsyncer_extension_*.{dylib,so}"]
    doc.install "README.md"
  end

  def caveats
    <<~EOS
      Enable only the native extensions you trust:
        syncer extension install #{lib}/syncer/libsyncer_extension_json.#{OS.mac? ? "dylib" : "so"}
    EOS
  end

  test do
    (testpath/"policy.hcl").write <<~HCL
      schema_version = 1
      name = "test"
      rule "file" {
        target = "result.txt"
        kind = "file"
        operation = "replace"
        value = "synced"
      }
    HCL
    system bin/"syncer", "--state-dir", testpath/"state", "add", "test", testpath/"policy.hcl"
    system bin/"syncer", "--state-dir", testpath/"state", "apply", "--dry-run"
    refute_path_exists testpath/"result.txt"
    system bin/"syncer", "--state-dir", testpath/"state", "apply"
    assert_equal "synced", (testpath/"result.txt").read
  end
end''')
