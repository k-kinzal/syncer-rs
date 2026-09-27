#!/usr/bin/env python3
"""Publish crates in dependency order; reruns skip exactly the already-published version."""
import json
import subprocess
import time
import urllib.error
import urllib.request

version = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"]))["packages"][0]["version"]
packages = ["syncer-language", "syncer-extension-sdk", "syncer-core", "syncer-cli", "syncer-extension-http", "syncer-extension-git", "syncer-extension-google-drive", "syncer-extension-json", "syncer-extension-claude"]
for package in packages:
    url = f"https://crates.io/api/v1/crates/{package}/{version}"
    try:
        urllib.request.urlopen(url)
        print(f"{package} {version} is already published")
        continue
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
    subprocess.run(["cargo", "publish", "--locked", "--package", package], check=True)
    for attempt in range(30):
        try:
            urllib.request.urlopen(url)
            break
        except urllib.error.HTTPError as error:
            if error.code != 404:
                raise
            time.sleep(2)
    else:
        raise RuntimeError(f"Registry did not expose {package} {version}")
