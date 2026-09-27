# Encrypted compliance reporting

Reporting is opt-in and separate from policy content. A downloaded policy cannot opt a user in, change the recipient, choose credentials or redirect a report.

Provider setup:

```sh
syncer report keygen ./provider.agekey
# Prints only the age public recipient. Protect provider.agekey; never distribute it.
```

Client enrollment:

```sh
syncer report enable company ./encrypted-reports --recipient age1...
# Or an endpoint accepting encrypted HTTP POST requests (requires the HTTP extension):
syncer report enable company https://reports.example.com/ingest --recipient age1... \
  --extension http --credential access_token=SYNCER_HTTP_ACCESS_TOKEN
syncer apply
syncer report list
syncer report flush
syncer report disable company
```

Enrollment pins the provider public key locally. Changing it requires disabling and re-enabling reporting. Payloads contain schema version, a scoped pseudonym, Unix observation time, rule IDs, policy name, content digest and compliance before/after the run. Status for inherited constrained rules is also reported to the defining policy after a descendant override. Reports deliberately omit paths, file content, desired/current values, reasons, contact addresses, credentials, usernames and hostnames.

The installation secret is randomly generated and kept in private local enrollment. A subject hash includes that secret, policy name and recipient key. This supports longitudinal counts within a provider scope without providing a global cross-provider device ID. It is pseudonymous, not mathematically anonymous: rare policy selections, timestamps, account identity and network metadata may still identify a person. Readers of the report store cannot decrypt ciphertext without the provider's private key, but can see names, ciphertext sizes and upload metadata. age uses fresh encryption randomness for each payload.

Dry-run never reports. Applied runs report actual before/after status. An unresolved non-mutating audit reports unchanged actual state. Fetch/parse errors, inherited-policy conflicts and filesystem write failures abort without a success report; absence of a report must be treated as unknown, never compliant.

Reports are encrypted before being written to `outbox/<policy>/*.age`. Successful delivery removes the local ciphertext; transport failures retain it for the next apply or `report flush`. Delivery is at least once: a crash after remote acceptance but before local deletion can produce duplicate ciphertext uploads. Provider aggregation groups observations by scoped subject/rule/revision instead of counting uploaded files as devices. Disabling reporting stops delivery; historical ciphertext is retained locally. Before re-enrolling a policy with a different provider, archive or remove its old outbox deliberately.

Provider analysis, after downloading encrypted blobs from the sink:

```sh
syncer report summarize ./downloaded-reports --identity ./provider.agekey
```

The result gives per-rule/per-revision observed device counts, latest compliant/violating counts and maximum observed resolution seconds. Elapsed time starts at the first observed violation, not the rule's installation time. A device repaired on its first observation records zero seconds. Nonreporting devices and actual rollout denominators require external fleet enrollment; `all_observed_compliant` does not mean the entire enterprise is compliant. Reports are not signed device attestations; possession of a public encryption key does not prove an uploader is trusted. Future Cloud ingestion must enforce authentication, replay policy, enrollment and retention separately.
