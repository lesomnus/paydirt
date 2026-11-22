use crate::{Result, generate, payday::Schema};
use heck::ToSnakeCase;
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn add(args: &[String]) -> Result<()> {
    let mut positional = Vec::new();
    let mut file = None;
    let mut global = false;
    let mut tenanted = false;
    let mut watch = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--global" => global = true,
            "--tenanted" => tenanted = true,
            "--watch" => watch = true,
            "--tenant" => {
                return Err(
                    "Payday supplies Tenant. Use --tenanted for an entity owned by a tenant".into(),
                );
            }
            "--file" => file = Some(args.next().ok_or("--file needs a filename")?.clone()),
            v if !v.starts_with('-') => positional.push(v),
            _ => return Err(format!("unknown entity option {arg}").into()),
        }
    }
    if global && tenanted {
        return Err("choose one of --global and --tenanted".into());
    }
    if !(1..=2).contains(&positional.len()) {
        return Err(crate::HELP.into());
    }
    let name = positional[0];
    if !name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
        || !name.bytes().all(|c| c.is_ascii_alphanumeric())
    {
        return Err("expected a capitalized protobuf message name".into());
    }
    let root = Path::new(positional.get(1).copied().unwrap_or(".")).canonicalize()?;
    let lock = fs::File::open(root.join("buf.yaml"))?;
    lock.lock()?;
    let schema = Schema::read(&root)?.ok_or("missing paydirt.schema.json")?;
    let work = tempfile::tempdir()?;
    generate::build(&root, work.path())?;
    let descriptor: Value = serde_json::from_slice(&fs::read(work.path().join("resolved.json"))?)?;
    let mut domain = 6;
    let mut used = std::collections::BTreeSet::new();
    for file in descriptor["file"].as_array().ok_or("missing descriptors")? {
        for message in file["messageType"].as_array().into_iter().flatten() {
            if file["package"] == schema.package && message["name"] == name {
                return Err(format!("{name} already exists").into());
            }
            let entity = &message["options"]["[payday.entity]"];
            if let Some(n) = entity["domain"].as_u64() {
                used.insert(n);
                if n <= 247
                    && (entity["own"].is_null()
                        || matches!(entity["own"].as_str(), Some("OWN_EVENT" | "OWN_FACT")))
                {
                    domain = domain.max(n);
                }
            }
        }
    }
    domain += 1;
    while used.contains(&domain) {
        domain += 1;
    }
    if domain > 247 {
        return Err("no application domain available below 248".into());
    }
    let mut file = PathBuf::from(file.unwrap_or_else(|| name.to_snake_case()));
    if file.components().count() != 1 || file.file_name().is_none() {
        return Err("--file must name a source file in the application package".into());
    }
    if file.extension().is_none() {
        file.set_extension("proto");
    }
    if file.extension().is_none_or(|e| e != "proto") || file.to_string_lossy().ends_with(".g.proto")
    {
        return Err("expected a source .proto file".into());
    }
    let package_path = schema.package.replace('.', "/");
    let dir = root.join("proto").join(&package_path);
    fs::create_dir_all(&dir)?;
    let path = dir.join(file);
    let mut source = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        format!(
            "edition = \"2023\";\npackage {};\noption features.field_presence = IMPLICIT;\noption go_package = {:?};\n",
            schema.package, schema.go_package
        )
    };
    let mut imports = vec![
        "orm.proto".to_string(),
        "payday.proto".into(),
        "google/protobuf/timestamp.proto".into(),
    ];
    if !global {
        imports.push(format!("{package_path}/payday/tenant.proto"));
    }
    for import in imports {
        let decl = format!("import {import:?};");
        if !source.contains(&decl) {
            source.push_str(&format!("\n{decl}\n"));
        }
    }
    let tenant = if global {
        ""
    } else {
        "  Tenant tenant = 2 [(orm.edge)={immutable:true}];\n"
    };
    let scope = if global { "global: {}" } else { "" };
    let mut index = String::new();
    let mut extra = "";
    let mut version = "";
    if watch {
        let prefix = if global {
            ""
        } else {
            "{name:\"tenant\",number:2},"
        };
        index = format!(
            "indexes: [{{name:\"page\",refs:[{prefix}{{name:\"date_created\",number:15}},{{name:\"id\",number:1}}]}}]"
        );
        extra = "list:{order:[{field:{name:\"date_created\"}},{field:{name:\"id\"}}],by:[{name:\"ref\"}],size:20,max:100} watch:{}";
        version = "  google.protobuf.Timestamp date_updated=13 [(orm.field)={version:{}}];\n";
    }
    source.push_str(&format!("\nmessage {name} {{\n  bytes id=1 [(orm.field)={{type:TYPE_UUID,key:true,default:\"\"}}];\n{tenant}  string alias=4;\n{version}  google.protobuf.Timestamp date_erased=14 [(orm.field)={{erased:{{}}}}];\n  google.protobuf.Timestamp date_created=15 [(orm.field)={{immutable:true,default:\"\"}}];\n  option (orm.message)={{rpc:{{crud:true}} {index}}};\n  option (payday.entity)={{domain:{domain} {scope} {extra}}};\n}}\n"));
    let mut temp = tempfile::NamedTempFile::new_in(&dir)?;
    temp.write_all(source.as_bytes())?;
    temp.persist(&path)?;
    println!("{}", path.display());
    Ok(())
}

pub fn doctor(root: &Path) -> Result<()> {
    crate::project::legacy_diagnostics(root)?;
    generate::run(root, true)?;
    crate::project::schema_advice(root)?;
    let checks = ["Cargo.toml", "src/lib.rs", "src/main.rs"];
    for path in checks {
        if !root.join(path).is_file() {
            return Err(format!("missing {path}").into());
        }
    }
    if root.join("sandbox/.paydirt-sandbox").is_file() {
        for path in [
            "sandbox/database/Cargo.toml",
            "sandbox/public/server.js",
            "sandbox/public/database.js",
            "sandbox/build.sh",
        ] {
            if !root.join(path).is_file() {
                return Err(format!("missing {path}").into());
            }
        }
    }
    let status = std::process::Command::new("cargo")
        .args(["check", "--quiet", "--all-targets"])
        .current_dir(root)
        .status()?;
    if !status.success() {
        return Err("application entry points or service layers do not compile".into());
    }
    println!("Schema, generated files and application entry points passed.");
    Ok(())
}
