schema_version = 1
name = "team"
contact = "team@example.com"
select_roles = ["company/developer"]

rule "sandbox-domains" {
  target = "claude://project/settings"
  kind = "json"
  operation = "array"
  pointer = "/sandbox/network/allowedDomains"
  override = "constrained"
  constraints {
    required_items = ["github.com", "registry.npmjs.org"]
    forbidden_items = ["retired.example.com"]
  }
  rationale = "This team uses npm in addition to company source hosting"
  added = "2026-09-27"
}
