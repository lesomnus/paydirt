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
    let image: Value = serde_json::from_slice(&fs::read(dest)?)?;
    let mut policies = BTreeMap::new();
    for file in image["file"].as_array().ok_or("expected files")? {
        let package = file["package"].as_str().unwrap_or_default();
        for message in file["messageType"].as_array().into_iter().flatten() {
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
    fs::write(
        work.join("generated/storage/policies.json"),
        serde_json::to_string_pretty(&policies)? + "\n",
    )?;
    Ok(())
}
