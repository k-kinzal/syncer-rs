//! Privacy boundary: only encrypted, allowlisted status leaves this module.
use crate::engine::RuleStatus;
use anyhow::{Context, Result};
use serde::Serialize;
use std::{
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};
#[derive(Serialize)]
struct Report<'a> {
    schema_version: u32,
    subject: String,
    observed_at: u64,
    rules: Vec<&'a RuleStatus>,
}
/// Stable only within a provider scope. Keep `installation_secret` private and random.
pub fn encrypt(
    recipient: &str,
    installation_secret: &str,
    scope: &str,
    status: &[RuleStatus],
) -> Result<Vec<u8>> {
    let key: age::x25519::Recipient = recipient
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid age recipient: {e}"))?;
    let subject = crate::digest(
        format!("syncer-report-v1\0{installation_secret}\0{scope}\0{recipient}").as_bytes(),
    );
    let report = Report {
        schema_version: 1,
        subject,
        observed_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        rules: status.iter().filter(|s| s.policy == scope).collect(),
    };
    let plain = serde_json::to_vec(&report)?;
    let encryptor = age::Encryptor::with_recipients(std::iter::once(&key as &dyn age::Recipient))
        .context("report encryption")?;
    let mut encrypted = vec![];
    let mut writer = encryptor.wrap_output(&mut encrypted)?;
    writer.write_all(&plain)?;
    writer.finish()?;
    Ok(encrypted)
}
/// Generate a provider identity. Store the secret privately; distribute only the recipient.
pub fn keygen() -> (String, String) {
    use age::secrecy::ExposeSecret;
    let identity = age::x25519::Identity::generate();
    (
        identity.to_string().expose_secret().to_string(),
        identity.to_public().to_string(),
    )
}
/// Decrypt downloaded reports and summarize latest compliance and observed resolution times.
pub fn summarize(identity: &str, encrypted: &[Vec<u8>]) -> Result<serde_json::Value> {
    use anyhow::ensure;
    use std::{collections::BTreeMap, io::Read};
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Envelope {
        schema_version: u32,
        subject: String,
        observed_at: u64,
        rules: Vec<RuleStatus>,
    }
    #[derive(Default)]
    struct Observation {
        latest: u64,
        compliant: bool,
        first_violation: Option<u64>,
        resolved: Option<u64>,
    }
    let key: age::x25519::Identity = identity
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid provider identity: {e}"))?;
    let mut reports = vec![];
    for bytes in encrypted {
        let reader = age::Decryptor::new(bytes.as_slice())?
            .decrypt(std::iter::once(&key as &dyn age::Identity))?;
        let mut plain = vec![];
        reader
            .take((syncer_extension_sdk::MAX_BYTES + 1) as u64)
            .read_to_end(&mut plain)?;
        ensure!(
            plain.len() <= syncer_extension_sdk::MAX_BYTES,
            "report plaintext exceeds size limit"
        );
        let report: Envelope = serde_json::from_slice(&plain)?;
        ensure!(report.schema_version == 1, "unsupported report version");
        reports.push(report);
    }
    reports.sort_by_key(|r| r.observed_at);
    let mut groups: BTreeMap<(String, String, String), BTreeMap<String, Observation>> =
        BTreeMap::new();
    for report in reports {
        for rule in report.rules {
            let observation = groups
                .entry((rule.policy, rule.id, rule.revision))
                .or_default()
                .entry(report.subject.clone())
                .or_default();
            if !rule.before && observation.first_violation.is_none() {
                observation.first_violation = Some(report.observed_at);
            }
            if rule.after && observation.first_violation.is_some() && observation.resolved.is_none()
            {
                observation.resolved = Some(report.observed_at);
            }
            if !rule.after {
                observation.resolved = None;
            }
            observation.latest = report.observed_at;
            observation.compliant = rule.after;
        }
    }
    Ok(
        serde_json::json!({"scope":"observed reporting devices only; not a fleet enrollment count","rules":groups.into_iter().map(|((policy,id,revision),subjects)|{
        let compliant=subjects.values().filter(|o|o.compliant).count();
        let max_seconds=subjects.values().filter_map(|o|Some(o.resolved?.saturating_sub(o.first_violation?))).max();
        serde_json::json!({"policy":policy,"id":id,"revision":revision,"observed_devices":subjects.len(),"compliant_devices":compliant,"violating_devices":subjects.len()-compliant,"all_observed_compliant":compliant==subjects.len(),"max_observed_resolution_seconds":max_seconds,"max_observed_resolution_days":max_seconds.map(|s|s as f64/86400.0),"latest_observation":subjects.values().map(|o|o.latest).max()})
    }).collect::<Vec<_>>()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_report_contains_no_cleartext() {
        let key = age::x25519::Identity::generate();
        let status = vec![RuleStatus {
            id: "rule-secret".into(),
            policy: "org".into(),
            revision: "r1".into(),
            before: false,
            after: true,
        }];
        let encrypted = encrypt(
            &key.to_public().to_string(),
            "private-random-id",
            "org",
            &status,
        )
        .unwrap();
        assert!(!String::from_utf8_lossy(&encrypted).contains("rule-secret"));
        let decryptor = age::Decryptor::new(encrypted.as_slice()).unwrap();
        let mut reader = decryptor
            .decrypt(std::iter::once(&key as &dyn age::Identity))
            .unwrap();
        let mut plain = String::new();
        std::io::Read::read_to_string(&mut reader, &mut plain).unwrap();
        assert!(plain.contains("rule-secret"));
        assert!(!plain.contains("private-random-id"));
    }
}
