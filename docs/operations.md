# Operations

Commands default to readable summaries and tables. Use `--output json` for
scripts, `--query` for JMESPath selection, and `--output jsonl` for continuous
daemon streams. See [output formats and queries](output.md).

## State and source enrollment

Default state is the platform config directory plus `syncer` (`~/Library/Application Support/syncer` on macOS, `$XDG_CONFIG_HOME/syncer` or `~/.config/syncer` on Linux, roaming AppData on Windows). Use `--state-dir` for a separate enrollment. It holds:

- `config.json`: ordered policy and asset sources, allowed target roots, native library digests, credential **environment variable names**, reporting recipients and a private installation secret.
- `cache/*.json`: last explicitly fetched/used source contents and revisions. `--asset` cache content is base64.
- `extensions/`: locally installed libraries.
- `backups/<transaction>/`: original files and transaction journal.
- `outbox/<policy>/`: encrypted undelivered reports.

`syncer list` shows sources; `syncer remove NAME` removes enrollment without undoing previously applied files. Re-enrollment is required to change local trust settings. Sources must have unique priorities; lower numbers are ancestors. `fetch` refreshes and validates the cache without applying targets. Offline use is explicit (`apply --offline`) and trusts the last cached policy, so do not use it for checking whether an enterprise policy has been revoked or updated.

Local assets and policies are read without network extensions. Asset example:

```sh
syncer add logo ./brand/logo.png --asset
syncer add policy ./policy.hcl
```

```hcl
schema_version = 1
name = "branding"
rule "logo" {
  target = "assets/logo.png"
  kind = "file"
  operation = "replace"
  content_from = "logo"
}
```

Whole-file assets may be binary. A downloaded policy can reference only explicitly enrolled asset names. Both assets and policies may use HTTP, Git or Drive transport. Pins (`add --sha256`) refer to decoded content, and block automatic updates until deliberately re-enrolled. Pushing to a pinned source is rejected when it would violate the pin. Active enrolled source files are protected from policy writes, preventing a lower-layer rule from rewriting its ancestor for the next cycle. Synchronize rule files into a staging path and explicitly publish with `push`. `push` is an explicit one-way conditional publish, not conflict-merging bidirectional sync. Local source revisions and HTTP/Drive If-Match prevent known stale publishes; transports without a usable revision are rejected.

## Background operation

`syncer daemon --interval 86400` refreshes and applies immediately, then every 24 hours after the previous cycle finishes. Failed cycles log errors and retry at that interval; they do not silently use cached policy. `daemon --once` is suitable for an external daily scheduler. Ctrl-C stops the foreground daemon during its waiting period. Each apply cycle uses the state lock, so local enrollment commands can run between cycles. Keep `--project` absolute and fixed; credentials must be supplied to the service environment.

Example Linux user service (`~/.config/systemd/user/syncer.service`, replace paths):

```ini
[Unit]
Description=Syncer file policy synchronization
After=network-online.target

[Service]
ExecStart=/home/USER/.cargo/bin/syncer --project /home/USER/project daemon --interval 86400
Restart=on-failure
RestartSec=60

[Install]
WantedBy=default.target
```

Enable with `systemctl --user daemon-reload` and `systemctl --user enable --now syncer`. A user manager must be running; configure login/linger according to your organization's policy.

Example macOS LaunchAgent (`~/Library/LaunchAgents/io.github.k-kinzal.syncer.plist`, replace paths):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>io.github.k-kinzal.syncer</string>
  <key>ProgramArguments</key><array>
    <string>/opt/homebrew/bin/syncer</string>
    <string>--project</string><string>/Users/USER/project</string>
    <string>daemon</string><string>--once</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>StartInterval</key><integer>86400</integer>
  <key>StandardOutPath</key><string>/Users/USER/Library/Logs/syncer.log</string>
  <key>StandardErrorPath</key><string>/Users/USER/Library/Logs/syncer-error.log</string>
</dict></plist>
```

Load with `launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/io.github.k-kinzal.syncer.plist`. Library/Logs should already exist. LaunchAgents inherit a different environment from interactive shells; configure OAuth environment variables explicitly, with suitable file permissions.

On Windows, create a Task Scheduler task under the intended user, triggered daily, with program `C:\path\syncer.exe` and arguments `--project C:\path\project daemon --once`. Set the working directory, failure retries and environment deliberately. It is a scheduler task, not a native Windows Service. This release does not install OS services automatically or elevate privileges.

## Backups and recovery

Every changing apply writes a backup transaction before replacing targets. The journal lists target paths, before/after hashes, numbered backups and whether the plan finished. Normal errors trigger rollback of already-written files if their content still matches Syncer's result. Inspect interrupted transactions and compare hashes before restoring files; preserve newer personal edits. For previously missing files, the backup field is null. Do not blindly delete them if another process has edited them after the interruption.

Transactions are per-file atomic, not globally atomic. Permissions on existing files are retained; timestamps, extended attributes, ACL replication and file ownership migration are not a cross-platform synchronization contract in v1. Backup cleanup and retention are manual. Protect the state directory: backups and policy caches can contain sensitive configuration even though outgoing telemetry is encrypted.
