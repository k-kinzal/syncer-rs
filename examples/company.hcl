schema_version = 1
name = "company"
contact = "developer-platform@example.com"
description = "Company baseline, activated by team role selection"
revision = "2026-09-27"

role "developer" {
  rule "sandbox-domains" {
    target = "settings.json"
    kind = "json"
    operation = "array"
    pointer = "/sandbox/network/allowedDomains"
    override = "constrained"
    constraints {
      required_items = ["github.com"]
      forbidden_items = ["retired.example.com"]
    }
    rationale = "Keep source access; block the retired service"
    added = "2026-09-27"
  }

  rule "sandbox-enabled" {
    target = "settings.json"
    kind = "json"
    operation = "set"
    pointer = "/sandbox/enabled"
    value = true
    override = "locked"
    rationale = "Sandbox is mandatory for company projects"
    added = "2026-09-27"
  }
}
