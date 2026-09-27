#!/usr/bin/env python3
"""Publish crates in dependency order; reruns skip exactly the already-published version."""
import json
import re
import subprocess
import time
import urllib.error
import urllib.request
from email.utils import parsedate_to_datetime

PACKAGES = ["syncer-language", "syncer-extension-sdk", "syncer-document", "syncer-core", "syncer-cli", "syncer-extension-http", "syncer-extension-git", "syncer-extension-google-drive", "syncer-extension-json", "syncer-extension-claude", "syncer-extension-structured"]


def is_published(url):
    try:
        with urllib.request.urlopen(url, timeout=30):
            return True
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
        return False


def retry_at(output):
    """Honor crates.io's explicit retry time only for a rate-limit response."""
    if "status 429" not in output:
        return None
    match = re.search(r"Please try again after ([^\r\n]+? GMT)", output)
    if match is None:
        return None
    try:
        return parsedate_to_datetime(match.group(1)).timestamp() + 2
    except (ValueError, TypeError, OverflowError):
        return None


def publish(package, version):
    url = f"https://crates.io/api/v1/crates/{package}/{version}"
    command = ["cargo", "publish", "--locked", "--package", package]
    for attempt in range(6):
        if is_published(url):
            print(f"{package} {version} is already published", flush=True)
            return
        result = subprocess.run(command, capture_output=True, text=True)
        output = result.stdout + result.stderr
        print(output, end="", flush=True)
        if result.returncode == 0:
            break
        deadline = retry_at(output)
        if deadline is None or not 0 < deadline - time.time() <= 3600 or attempt == 5:
            raise subprocess.CalledProcessError(result.returncode, command)
        print(f"{package}: respecting registry rate limit until {time.ctime(deadline)}", flush=True)
        while (remaining := deadline - time.time()) > 0:
            time.sleep(min(remaining, 60))

    for _ in range(30):
        if is_published(url):
            return
        time.sleep(2)
    raise RuntimeError(f"Registry did not expose {package} {version}")


def main():
    version = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"]))["packages"][0]["version"]
    for package in PACKAGES:
        publish(package, version)


if __name__ == "__main__":
    main()
