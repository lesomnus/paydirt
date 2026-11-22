//! Generate the language-independent half of a Payday schema with its original plugins.
use crate::{
    Result,
    generate::{files, write_file},
};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schema {
    pub(crate) package: String,
    pub(crate) go_package: String,
}

const OWN: &[(&str, &str)] = &[
    ("access", include_str!("../upstream/payday/access.proto")),
    (
        "archived",
        include_str!("../upstream/payday/archived.proto"),
    ),
    ("audit", include_str!("../upstream/payday/audit.proto")),
    ("holder", include_str!("../upstream/payday/holder.proto")),
    ("outbox", include_str!("../upstream/payday/outbox.proto")),
    ("owed", include_str!("../upstream/payday/owed.proto")),
    ("rowseal", include_str!("../upstream/payday/rowseal.proto")),
    ("tenant", include_str!("../upstream/payday/tenant.proto")),
];

impl Schema {
    pub fn read(root: &Path) -> Result<Option<Self>> {
        let path = root.join("paydirt.schema.json");
        if !path.try_exists()? {
            return Ok(None);
        }
        let schema: Self = serde_json::from_slice(&fs::read(path)?)?;
        if !schema.package.split('.').all(|p| {
            p.as_bytes()
                .first()
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
                && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        }) || schema.go_package.is_empty()
            || schema.go_package.contains(['"', '\\', '\n', '\r'])
        {
            return Err("invalid package or go_package in paydirt.schema.json".into());
        }
        Ok(Some(schema))
    }

    pub fn entities(&self, work: &Path, overlays: &mut BTreeSet<PathBuf>) -> Result<()> {
        static LIBRARY: include_dir::Dir =
            include_dir::include_dir!("$CARGO_MANIFEST_DIR/upstream/proto");
        let mut shipped = BTreeSet::new();
        fn library(
            dir: &include_dir::Dir,
            work: &Path,
            shipped: &mut BTreeSet<PathBuf>,
        ) -> Result<()> {
            for file in dir.files() {
                publish(work, file.path(), file.contents(), shipped)?;
            }
            for child in dir.dirs() {
                library(child, work, shipped)?;
            }
            Ok(())
        }

        // The first pass links the app's options. Only seed optional definitions
        // reachable by imports, so an unselected Event cannot shadow an app's Event.
        let source = work.join("proto");
        let mut imports = BTreeSet::new();
        let inputs = files(&source)?
            .into_keys()
            .map(|p| source.join(p))
            .collect::<Vec<_>>();
        for path in inputs
            .into_iter()
            .chain(overlays.iter().map(|p| work.join("ext").join(p)))
        {
            let text = output(
                crate::project::buf().arg("format").arg(path),
                "reading protobuf imports",
            )?;
            for line in text.lines().filter(|l| l.starts_with("import ")) {
                if let Some(name) = line.split('"').nth(1) {
                    imports.insert(name.to_owned());
                }
            }
        }
        library(&LIBRARY, work, &mut shipped)?;
        let dir = PathBuf::from(self.package.replace('.', "/")).join("payday");
        for &(name, source) in OWN {
            self.copy_entity(work, &dir, name, source, overlays, &mut shipped)?;
        }
        let mut seeds = Vec::new();
        if imports.contains(&format!("{}/notification_account.proto", dir.display())) {
            imports.insert(format!("{}/notification.proto", dir.display()));
        }
        for &(name, source, _) in OPTIONAL {
            let rel = dir.join(format!("{name}.proto"));
            if imports.contains(rel.to_str().unwrap()) {
                let path = work.join("proto").join(&rel);
                if path.exists() {
                    return Err(format!("reserved Payday entity path: {}", rel.display()).into());
                }
                write_file(&path, self.rewrite(source, &dir).as_bytes())?;
                seeds.push(path);
            }
        }
        let image = work.join("app-options.json");
        output(
            crate::project::buf()
                .current_dir(work)
                .args(["build", "--as-file-descriptor-set", "-o"])
                .arg(&image),
            "reading Payday app options",
        )?;
        let image: serde_json::Value = serde_json::from_slice(&fs::read(image)?)?;
        let mut app = None;
        for file in image["file"].as_array().ok_or("missing descriptor files")? {
            if let Some(options) = file["options"].get("[payday.app]")
                && (file["package"] != self.package || app.replace(options).is_some())
            {
                return Err(
                    "payday.app must be declared once in the configured app package".into(),
                );
            }
        }
        for path in seeds {
            fs::remove_file(path)?;
        }
        let app = app.unwrap_or(&serde_json::Value::Null);
        for (name, original) in [("access", 6), ("archived", 5)] {
            if let Some(domain) = app["domains"][name].as_u64().filter(|n| *n != 0) {
                let rel = dir.join(format!("{name}.proto"));
                let content = fs::read_to_string(work.join("proto").join(&rel))?;
                publish(
                    work,
                    &rel,
                    numbered(&content, original, domain)?.as_bytes(),
                    &mut shipped,
                )?;
            }
        }
        for &(name, source, group) in OPTIONAL {
            let Some(options) = app.get(group) else {
                continue;
            };
            let selected = match name {
                "notification_account" if options.get("accounts").is_none() => continue,
                "notification_account" => source.to_owned(),
                _ => {
                    let field = match name {
                        "notification_receipt" => "receipt",
                        "entitlement" => "domain",
                        other => other,
                    };
                    let n = options[field]
                        .as_u64()
                        .ok_or_else(|| format!("payday.app.{group}.{field} must name a domain"))?;
                    let s = numbered(source, 0, n)?;
                    if group == "notify" && options.get("global").is_some() {
                        global_notification(&s)?
                    } else {
                        s
                    }
                }
            };
            self.copy_entity(work, &dir, name, &selected, overlays, &mut shipped)?;
        }
        Ok(())
    }

    fn copy_entity(
        &self,
        work: &Path,
        dir: &Path,
        name: &str,
        source: &str,
        overlays: &mut BTreeSet<PathBuf>,
        shipped: &mut BTreeSet<PathBuf>,
    ) -> Result<()> {
        let overlay = PathBuf::from("payday").join(format!("{name}.ext.proto"));
        let content = if overlays.remove(&overlay) {
            let base = work.join("original.proto");
            fs::write(&base, source)?;
            merge(&base, &work.join("ext").join(overlay), true)?
        } else {
            source.to_owned()
        };
        let rel = dir.join(format!("{name}.proto"));
        publish(work, &rel, self.rewrite(&content, dir).as_bytes(), shipped)
    }

    fn rewrite(&self, source: &str, dir: &Path) -> String {
        let mut out = String::new();
        for line in source.lines() {
            if line.starts_with("package ") {
                out.push_str(&format!("package {};", self.package));
            } else if line.starts_with("option go_package = ") {
                out.push_str(&format!("option go_package = {:?};", self.go_package));
            } else if let Some(local) = line
                .strip_prefix("import \"payday/")
                .or_else(|| line.strip_prefix("import \"notify/"))
            {
                out.push_str(&format!(
                    "import \"{}/{local}",
                    dir.to_string_lossy().replace('\\', "/")
                ));
            } else {
                out.push_str(line);
            }
            out.push('\n');
        }
        out
    }
}

const OPTIONAL: &[(&str, &str, &str)] = &[
    (
        "event",
        include_str!("../upstream/past/event.proto"),
        "past",
    ),
    ("fact", include_str!("../upstream/past/fact.proto"), "past"),
    (
        "entitlement",
        include_str!("../upstream/entitle/entitlement.proto"),
        "entitlements",
    ),
    (
        "notification",
        include_str!("../upstream/notify/notification.proto"),
        "notify",
    ),
    (
        "notification_receipt",
        include_str!("../upstream/notify/notification_receipt.proto"),
        "notify",
    ),
    (
        "notification_account",
        include_str!("../upstream/notify/notification_account.proto"),
        "notify",
    ),
];

fn numbered(source: &str, original: u64, domain: u64) -> Result<String> {
    if !(1..=255).contains(&domain) {
        return Err(format!("domain {domain} must fit in a nonzero byte").into());
    }
    let line = format!("    domain: {original}\n");
    if source.matches(&line).count() != 1 {
        return Err("unexpected entity domain declaration".into());
    }
    Ok(source.replace(&line, &format!("    domain: {domain}\n")))
}

// Only the storage record changes scope. The public request selectors keep tenant_id.
fn global_notification(source: &str) -> Result<String> {
    let column = "  bytes tenant_id = 2 [(orm.field) = {type: TYPE_UUID}];\n";
    let index = "          {\n            name: \"tenant_id\"\n            number: 2\n          }";
    let tenant = "tenanted: {field: \"tenant_id\"}";
    if source.matches(column).count() != 1 || source.matches(tenant).count() != 1 {
        return Err("unexpected notification storage definition".into());
    }
    Ok(source
        .replace(column, "")
        .replace(&format!("{index},\n"), "")
        .replace(&format!("{index}\n"), "")
        .replace(tenant, "global: {}"))
}

fn publish(work: &Path, rel: &Path, bytes: &[u8], shipped: &mut BTreeSet<PathBuf>) -> Result<()> {
    let path = work.join("proto").join(rel);
    if shipped.insert(rel.to_owned()) && path.exists() {
        return Err(format!("app source collides with Payday schema: {}", rel.display()).into());
    }
    write_file(&path, bytes)?;
    write_file(&work.join("generated/proto").join(rel), bytes)
}

fn output(command: &mut Command, operation: &str) -> Result<String> {
    let result = command.output().map_err(|e| format!("{operation}: {e}"))?;
    if !result.status.success() {
        return Err(format!("{operation}: {}", String::from_utf8_lossy(&result.stderr)).into());
    }
    Ok(String::from_utf8(result.stdout)?)
}

fn methods(text: &str) -> BTreeSet<&str> {
    text.lines()
        .filter_map(|l| l.trim_start().strip_prefix("rpc "))
        .filter_map(|l| l.split(['(', ' ']).next())
        .collect()
}

fn merge(base: &Path, overlay: &Path, user: bool) -> Result<String> {
    if user {
        // Parse/format first: overlays are fragments and may refer to types in the base.
        let text = output(
            crate::project::buf().arg("format").arg(overlay),
            "parsing schema overlay",
        )?;
        if text
            .lines()
            .any(|line| line.starts_with("option features."))
        {
            return Err(format!("{} sets file-wide features", overlay.display()).into());
        }
        let current = fs::read_to_string(base)?;
        let previous = methods(&current);
        for name in methods(&text) {
            if previous.contains(name) {
                return Err(format!("{} redefines generated RPC {name}", overlay.display()).into());
            }
        }
    }
    output(
        Command::new("protobuf-merge")
            .arg("-strict")
            .arg(base)
            .arg(overlay),
        "merging schema overlay",
    )
}

pub fn contracts(work: &Path, overlays: &mut BTreeSet<PathBuf>) -> Result<()> {
    let standard = work.join("generated/proto");
    let additions = work.join("payday-additions");
    let extensions = if additions.exists() {
        files(&additions)?
    } else {
        Default::default()
    };
    for (rel, data) in &extensions {
        if let Some(stem) = rel
            .to_str()
            .and_then(|s| s.strip_suffix("_svc.resource.proto"))
        {
            let path = standard.join(format!("{stem}_svc.g.proto"));
            if !path.exists() {
                let text = std::str::from_utf8(data)?;
                let (header, _) = text
                    .split_once("\nservice ")
                    .ok_or("missing computed resource service")?;
                write_file(&path, header.as_bytes())?;
            }
        }
    }
    for rel in files(&standard)?.into_keys() {
        let Some(stem) = rel.to_str().and_then(|s| s.strip_suffix("_svc.g.proto")) else {
            continue;
        };
        let path = standard.join(&rel);
        for suffix in ["_svc.pd.proto", "_svc.resource.proto"] {
            let ext = additions.join(format!("{stem}{suffix}"));
            if ext.exists() {
                fs::write(&path, merge(&path, &ext, false)?)?;
            }
        }
        let ext = PathBuf::from(format!("{stem}_svc.ext.proto"));
        if overlays.remove(&ext) {
            fs::write(&path, merge(&path, &work.join("ext").join(ext), true)?)?;
        }
    }
    if let Some(orphan) = overlays.first() {
        return Err(format!("no generated target for overlay {}", orphan.display()).into());
    }
    Ok(())
}
