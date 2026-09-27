schema_version = 1
name = "developer-formats"
description = "Selected settings for a disposable project; requires the structured extension"

rule "cargo-jobs" {
  target = ".cargo/config.toml"
  kind = "toml"
  operation = "ensure"
  pointer = "/build/jobs"
  value = 1
  override = "constrained"
  constraints { min = 1 }
  rationale = "Keep a developer's preferred parallelism when it is valid"
}

rule "required-domains" {
  target = "settings.yaml"
  kind = "yaml"
  operation = "array"
  pointer = "/allowedDomains"
  override = "constrained"
  constraints {
    required_items = ["github.com"]
    forbidden_items = ["retired.example.com"]
  }
  rationale = "Preserve personal domains while enforcing team requirements"
}

rule "local-env" {
  target = ".env.example"
  kind = "dotenv"
  operation = "set"
  pointer = "/SYNCER_EXAMPLE"
  value = "enabled"
}
