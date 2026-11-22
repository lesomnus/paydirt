use crate::Result;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

pub fn generate(work: &Path) -> Result<()> {
    let dest = work.join("resolved.json");
    let status = crate::project::buf()
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
    let status = crate::project::buf()
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
            let mut config = app.clone();
            config["goPackage"] = file["options"]["goPackage"].clone();
            apps.insert(package, config);
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
    let mut messages = BTreeMap::new();
    fn describe(
        prefix: &str,
        msg: &Value,
        out: &mut BTreeMap<String, Value>,
        policies: &BTreeMap<String, Value>,
    ) {
        let Some(short) = msg["name"].as_str() else {
            return;
        };
        let name = if prefix.is_empty() {
            short.into()
        } else {
            format!("{prefix}.{short}")
        };
        let mut fields = Vec::new();
        // Generated CRUD request fields inherit the entity's secret declarations.
        let entity = policies
            .keys()
            .filter(|e| {
                name == **e
                    || name
                        .strip_prefix(e.as_str())
                        .is_some_and(|s| matches!(s, "AddRequest" | "PatchRequest" | "Ref"))
            })
            .max_by_key(|e| e.len());
        for f in msg["field"].as_array().into_iter().flatten() {
            let secret = f["options"]["[payday.field]"]["secret"] == true
                || entity.and_then(|e| policies.get(e)).is_some_and(|p| {
                    p["fields"][f["name"].as_str().unwrap_or_default()]["secret"] == true
                });
            let target = if f["type"] == "TYPE_MESSAGE" {
                f["typeName"]
                    .as_str()
                    .unwrap_or_default()
                    .trim_start_matches('.')
            } else {
                ""
            };
            fields.push(
                json!({"name":f["jsonName"],"number":f["number"],"target":target,"secret":secret}),
            );
        }
        out.insert(
            name.clone(),
            json!({"fields":fields,"map":msg["options"]["mapEntry"]==true}),
        );
        for nested in msg["nestedType"].as_array().into_iter().flatten() {
            describe(&name, nested, out, policies);
        }
    }
    for file in image["file"].as_array().unwrap() {
        for msg in file["messageType"].as_array().into_iter().flatten() {
            describe(
                file["package"].as_str().unwrap_or_default(),
                msg,
                &mut messages,
                &policies,
            );
        }
    }
    fs::write(
        work.join("generated/storage/admission.json"),
        serde_json::to_vec(&messages)?,
    )?;
    fs::write(
        work.join("generated/storage/policies.json"),
        serde_json::to_string_pretty(&policies)? + "\n",
    )?;
    Ok(())
}
