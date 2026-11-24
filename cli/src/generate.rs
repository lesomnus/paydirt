use crate::{Result, payday};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const MARKER: &str = ".paydirt-generated";
const STAMP: &[u8] = b"Owned by paydirt gen. Edit the source files in proto/ instead.\n";

pub(crate) fn files(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    let mut out = BTreeMap::new();
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let ty = entry.file_type()?;
            if ty.is_dir() {
                visit(root, &path, out)?;
            } else if ty.is_file() {
                out.insert(path.strip_prefix(root)?.into(), fs::read(path)?);
            } else {
                return Err(format!("unsupported file type: {}", path.display()).into());
            }
        }
        Ok(())
    }
    visit(root, root, &mut out)?;
    Ok(out)
}

pub(crate) fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(path, bytes)?;
    Ok(())
}

fn buf(work: &Path, template: &str) -> Result<()> {
    fs::write(work.join("buf.gen.yaml"), template)?;
    let status = crate::project::buf()
        .args(["generate", "--template", "buf.gen.yaml"])
        .current_dir(work)
        .status()
        .map_err(|e| format!("starting buf generate: {e}"))?;
    if !status.success() {
        return Err(format!("buf generate failed ({status})").into());
    }
    Ok(())
}

pub(crate) fn build(root: &Path, work: &Path) -> Result<()> {
    for name in ["buf.yaml", "buf.lock"] {
        let src = root.join(name);
        if name == "buf.yaml" || src.exists() {
            fs::copy(src, work.join(name))?;
        }
    }
    let schema = payday::Schema::read(root)?;
    let mut overlays = std::collections::BTreeSet::new();
    for (name, bytes) in files(&root.join("proto"))? {
        if name.extension().is_none_or(|ext| ext != "proto") {
            continue;
        }
        if let Ok(rel) = name.strip_prefix("ext") {
            if schema.is_none() {
                return Err("schema overlays require paydirt.schema.json".into());
            }
            overlays.insert(rel.to_owned());
            write_file(&work.join("ext").join(rel), &bytes)?;
        } else {
            write_file(&work.join("proto").join(name), &bytes)?;
        }
    }
    let mut template = include_str!("../templates/contracts.yaml").to_owned();
    if let Some(schema) = &schema {
        schema.entities(work, &mut overlays)?;
        template.push_str("  - local: protoc-gen-pd\n    out: payday-additions\n    strategy: all\n    opt: [stage=proto]\n");
    }
    buf(work, &template)?;
    if schema.is_some() {
        payday::contracts(work, &mut overlays)?;
    }
    let contracts = work.join("generated/proto");
    fs::create_dir_all(&contracts)?;
    for (name, bytes) in files(&contracts)? {
        let dest = work.join("proto").join(&name);
        // The schema pass already installed the owned entities and option definitions.
        if !name.to_string_lossy().ends_with("_svc.g.proto") {
            continue;
        }
        if dest.exists() {
            return Err(format!(
                "generated contract collides with source: {}",
                name.display()
            )
            .into());
        }
        write_file(&dest, &bytes)?;
    }
    buf(work, include_str!("../templates/bindings.yaml"))?;
    let storage = work.join("generated/storage");
    let model = paydirt_model::Model::parse(&fs::read(storage.join("model.json"))?)?;
    for (file, dialect) in [
        ("postgres.sql", paydirt_model::Dialect::Postgres),
        ("sqlite.sql", paydirt_model::Dialect::Sqlite),
    ] {
        fs::write(storage.join(file), model.ddl(dialect)?)?;
    }
    fs::write(storage.join("tables.rs"), model.diesel_tables()?)?;
    if schema.is_some() {
        crate::policy::generate(work)?;
        crate::layers::generate(&work.join("generated/services"))?;
        crate::server::generate(work)?;
        crate::sinks::generate(work)?;
    }
    fs::write(work.join("generated").join(MARKER), STAMP)?;
    Ok(())
}

// protobuf-es emits descriptors for Connect clients, while Payday emits the
// domain, entity and event tables from the very same resolved descriptors.
fn typescript(root: &Path, work: &Path) -> Result<()> {
    let plugin = [
        "ts/node_modules/.bin/protoc-gen-es",
        "node_modules/.bin/protoc-gen-es",
    ]
    .into_iter()
    .map(|p| root.join(p))
    .find(|p| p.is_file())
    .unwrap_or_else(|| PathBuf::from("protoc-gen-es"));
    let template = format!(
        r#"version: v2
plugins:
  - local: {}
    out: generated/ts
    include_imports: true
    strategy: all
    opt: [target=ts, import_extension=js]
  - local: protoc-gen-pd
    out: generated/ts
    include_imports: true
    strategy: all
    opt: [stage=ts]
"#,
        serde_json::to_string(&plugin)?
    );
    buf(work, &template).map_err(|e| {
        format!("generating TypeScript (install @bufbuild/protoc-gen-es in ts/ or the app): {e}")
    })?;
    let output = work.join("generated/ts");
    for (path, bytes) in files(&output)? {
        let source = String::from_utf8(bytes)?;
        let imports = source.replace("_svc.g_pb.js\"", "_svc_pb.js\"");
        let name = path
            .file_name()
            .unwrap()
            .to_str()
            .ok_or("non-UTF-8 generated filename")?;
        if let Some(stem) = name.strip_suffix("_svc.g_pb.ts") {
            let dest = output.join(path.with_file_name(format!("{stem}_svc_pb.ts")));
            if dest.exists() {
                return Err(format!(
                    "generated TypeScript filename collision: {}",
                    dest.display()
                )
                .into());
            }
            fs::rename(output.join(&path), &dest)?;
            fs::write(dest, imports)?;
        } else if imports != source {
            fs::write(output.join(path), imports)?;
        }
    }
    Ok(())
}

fn owned(dir: &Path) -> Result<bool> {
    match dir.symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Ok(meta) if meta.is_dir() && fs::read(dir.join(MARKER)).as_deref().ok() == Some(STAMP) => {
            Ok(true)
        }
        Ok(_) => Err(format!(
            "{} already exists and is not owned by paydirt gen",
            dir.display()
        )
        .into()),
        Err(e) => Err(e.into()),
    }
}

pub fn run(root: &Path, check: bool) -> Result<()> {
    run_with_ts(root, check, false)
}

pub fn run_with_ts(root: &Path, check: bool, ts: bool) -> Result<()> {
    let root = root.canonicalize()?;
    let config = root.join("buf.yaml");
    let lock = fs::File::open(&config).map_err(|e| format!("{}: {e}", config.display()))?;
    // Lock the stable project input, so read-only checks never need to create a lockfile.
    if check {
        lock.lock_shared()?;
    } else {
        lock.lock()?;
    }
    let output = root.join("generated");
    let backup = root.join(".paydirt-previous");
    if owned(&backup)? {
        if check {
            return Err("interrupted generation; run paydirt gen to finish recovery".into());
        }
        if owned(&output)? {
            fs::remove_dir_all(&backup)?;
        } else {
            fs::rename(&backup, &output)?;
        }
    }
    let exists = owned(&output)?;
    let work = if check {
        tempfile::tempdir()?
    } else {
        tempfile::tempdir_in(&root)?
    };
    build(&root, work.path())?;
    if ts || output.join("ts").is_dir() {
        typescript(&root, work.path())?;
    }
    let fresh = work.path().join("generated");
    let current = if exists {
        files(&output)?
    } else {
        BTreeMap::new()
    };
    let next = files(&fresh)?;
    if next == current {
        return Ok(());
    }
    if check {
        let changed = next
            .keys()
            .chain(current.keys())
            .find(|p| next.get(*p) != current.get(*p))
            .unwrap();
        return Err(format!(
            "generated/{} is out of date; run paydirt gen",
            changed.display()
        )
        .into());
    }
    if exists {
        fs::rename(&output, &backup)?;
    }
    if let Err(err) = fs::rename(&fresh, &output) {
        if exists {
            fs::rename(&backup, &output)?;
        }
        return Err(err.into());
    }
    if exists {
        fs::remove_dir_all(&backup)?;
    }
    Ok(())
}
