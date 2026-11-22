use crate::Result;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, process::Command};

pub fn generate(work: &Path) -> Result<()> {
    let dest = work.join("resolved.json");
    let status = Command::new("buf")
        .current_dir(work)
        .args([
            "build",
            "--as-file-descriptor-set",
            "--exclude-source-info",
            "-o",
        ])
        .arg(&dest)
        .status()?;
    if !status.success() {
        return Err("could not resolve application policies".into());
    }
    let binary = work.join("generated/storage/descriptors.binpb");
    let status = Command::new("buf")
        .current_dir(work)
        .args([
            "build",
            "--as-file-descriptor-set",
            "--exclude-source-info",
            "-o",
        ])
        .arg(binary)
        .status()?;
    if !status.success() {
        return Err("failed to export RPC descriptors".into());
    }
    let image: Value = serde_json::from_slice(&fs::read(dest)?)?;
    let mut policies = BTreeMap::new();
    let mut apps = BTreeMap::new();
    let mut event_types = BTreeMap::new();
    let mut payloads = Vec::new();
    for file in image["file"].as_array().ok_or("expected files")? {
        let package = file["package"].as_str().unwrap_or_default();
        if let Some(app) = file["options"].get("[payday.app]") {
            apps.insert(package, app);
        }
        for message in file["messageType"].as_array().into_iter().flatten() {
            if message["options"].get("[payday.notification]").is_some() {
                payloads.push(format!(
                    "{package}.{}",
                    message["name"].as_str().ok_or("missing payload name")?
                ));
            }
            if let Some(kinds) = message["options"]["[payday.event]"]["kinds"].as_array() {
                for kind in kinds {
                    let kind = kind.as_str().ok_or("invalid event kind")?;
                    let name = message["name"].as_str().ok_or("missing event type")?;
                    if event_types
                        .insert(kind, format!("{package}.{name}"))
                        .is_some()
                    {
                        return Err(format!("duplicate event kind {kind}").into());
                    }
                }
            }
            let Some(entity) = message["options"].get("[payday.entity]") else {
                continue;
            };
            let name = message["name"].as_str().ok_or("entity name")?;
            let mut fields = BTreeMap::new();
            for field in message["field"].as_array().into_iter().flatten() {
                if let Some(value) = field["options"].get("[payday.field]") {
                    fields.insert(field["name"].as_str().ok_or("field name")?, value);
                }
            }
            policies.insert(
                format!("{package}.{name}"),
                json!({"entity":entity,"fields":fields,"methods":{}}),
            );
        }
    }
    for file in image["file"].as_array().unwrap() {
        let package = file["package"].as_str().unwrap_or_default();
        for service in file["service"].as_array().into_iter().flatten() {
            if let Some(short) = service["name"]
                .as_str()
                .and_then(|s| s.strip_suffix("Service"))
                && matches!(short, "Notification" | "NotificationAccount")
                && apps.get(package).is_some_and(|a| a.get("notify").is_some())
                && file["name"].as_str().is_some_and(|p| {
                    p.ends_with("/payday/notification.proto")
                        || p.ends_with("/payday/notification_account.proto")
                })
            {
                policies.insert(
                    format!("{package}.{short}"),
                    json!({"notification":true,"methods":{}}),
                );
            }
            let Some(entity) = service["name"]
                .as_str()
                .and_then(|s| s.strip_suffix("Service"))
                .and_then(|s| policies.get_mut(&format!("{package}.{s}")))
            else {
                continue;
            };
            for method in service["method"].as_array().into_iter().flatten() {
                let name = method["name"].as_str().ok_or("missing method name")?;
                entity["methods"][name] = json!({"input":method["inputType"],"output":method["outputType"],"policy":method["options"]["[payday.method]"],"client_stream":method["clientStreaming"]==true,"server_stream":method["serverStreaming"]==true});
            }
        }
    }
    for (name, policy) in &mut policies {
        if policy["entity"]["own"] == "OWN_HOLDER"
            && let Some(app) = name.rsplit_once('.').and_then(|(p, _)| apps.get(p))
        {
            policy["application"] = (*app).clone();
            if !payloads.is_empty() {
                policy["application"]["notificationPayloads"] = json!(payloads);
            }
            if !event_types.is_empty() {
                policy["application"]["eventTypes"] = json!(event_types);
            }
        }
    }
    fs::write(
        work.join("generated/storage/policies.json"),
        serde_json::to_string_pretty(&policies)? + "\n",
    )?;
    Ok(())
}
