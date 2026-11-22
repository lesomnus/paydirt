use crate::{Result, generate, payday};
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use toml_edit::{DocumentMut, Item, TableLike};

pub(crate) fn buf() -> Command {
    Command::new(std::env::var_os("PAYDIRT_BUF").unwrap_or_else(|| "buf".into()))
}

pub fn forward(root: &Path, args: &[String]) -> Result<()> {
    if !root.join("buf.yaml").is_file() {
        return Err("project has no buf.yaml".into());
    }
    let status = buf().current_dir(root).args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("buf exited with {status}").into())
    }
}

pub fn entities(root: &Path) -> Result<()> {
    if payday::Schema::read(root)?.is_none() {
        return Err("missing paydirt.schema.json".into());
    }
    let work = tempfile::tempdir()?;
    generate::build(root, work.path())?;
    let policies: std::collections::BTreeMap<String, Value> = serde_json::from_slice(&fs::read(
        work.path().join("generated/storage/policies.json"),
    )?)?;
    println!("DOMAIN\tENTITY\tSCOPE");
    for (name, p) in policies {
        let e = &p["entity"];
        if let Some(domain) = e["domain"].as_u64() {
            let scope = if e.get("global").is_some() {
                "global"
            } else if e.get("tenant").is_some() {
                "tenant"
            } else {
                "tenanted"
            };
            println!("{domain}\t{name}\t{scope}");
        }
    }
    Ok(())
}

struct Edit {
    path: PathBuf,
    before: String,
    after: String,
}
struct Version {
    change: Option<String>,
    name: String,
    current: String,
    target: String,
}
#[derive(Default)]
struct Report {
    rows: Vec<Version>,
    edits: Vec<Edit>,
}

const APP: &str = include_str!("../templates/app/Cargo.toml.in");
const WORKER: &str = include_str!("../templates/sandbox/database/Cargo.toml");
const PAGE: &[(&str, &str, &str)] = &[
    ("@bufbuild/protobuf", "2.12.1", "2.16.0"),
    ("@bufbuild/protoc-gen-es", "2.12.1", "2.16.0"),
    ("@connectrpc/connect", "2.1.2", "2.3.0"),
    ("@connectrpc/connect-web", "2.1.2", "2.3.0"),
    ("@lesomnus/payday", "0.1.2", "0.1.2"),
    ("typescript", "5.9.3", "5.9.3"),
    ("vite", "7.2.0", "8.3.6"),
    ("@vitejs/plugin-react", "5.0.4", "5.0.4"),
];
fn dep_value<'a>(item: &'a Item, key: &str) -> Option<&'a str> {
    item.as_str()
        .filter(|_| key == "version")
        .or_else(|| item.get(key).and_then(Item::as_str))
}

// Retain custom sources, features and app-owned dependencies. Align the dependencies
// whose exact releases or revisions the generated runtime was built against.
fn cargo_group(
    actual: &mut dyn TableLike,
    template: &dyn TableLike,
    rows: &mut Vec<Version>,
    label: &str,
) {
    for (name, reference) in template.iter() {
        let Some(dep) = actual.get_mut(name) else {
            continue;
        };
        let (key, expected) = if let Some(rev) = dep_value(reference, "rev") {
            ("rev", rev)
        } else if let Some(v) = dep_value(reference, "version").filter(|s| s.starts_with('=')) {
            ("version", v)
        } else {
            continue;
        };
        if dep.get("path").is_some()
            || dep.get("git").and_then(Item::as_str) != reference.get("git").and_then(Item::as_str)
        {
            continue;
        }
        let Some(version) = dep_value(dep, key) else {
            continue;
        };
        rows.push(Version {
            change: (version != expected).then(|| expected.to_owned()),
            name: format!("{label} {name}"),
            current: version.to_owned(),
            target: expected.to_owned(),
        });
        if version != expected {
            if dep.is_str() {
                *dep = toml_edit::value(expected);
            } else if let Some(table) = dep.as_table_like_mut() {
                table.insert(key, toml_edit::value(expected));
            }
        }
    }
}
fn cargo(report: &mut Report, path: PathBuf, template: &str) -> Result<()> {
    let before = fs::read_to_string(&path)?;
    let mut doc = before.parse::<DocumentMut>()?;
    let target = template.parse::<DocumentMut>()?;
    let label = path.display().to_string();
    if let (Some(a), Some(b)) = (
        doc.get_mut("dependencies")
            .and_then(Item::as_table_like_mut),
        target.get("dependencies").and_then(Item::as_table_like),
    ) {
        cargo_group(a, b, &mut report.rows, &label);
    }
    if let Some(targets) = target.get("target").and_then(Item::as_table_like) {
        for (cfg, t) in targets.iter() {
            if let (Some(a), Some(b)) = (
                doc.get_mut("target")
                    .and_then(|d| d.get_mut(cfg))
                    .and_then(|d| d.get_mut("dependencies"))
                    .and_then(Item::as_table_like_mut),
                t.get("dependencies").and_then(Item::as_table_like),
            ) {
                cargo_group(a, b, &mut report.rows, &format!("{label} {cfg}"));
            }
        }
    }
    let after = doc.to_string();
    if before != after {
        report.edits.push(Edit {
            path,
            before,
            after,
        });
    }
    Ok(())
}
fn page(report: &mut Report, path: PathBuf) -> Result<()> {
    let before = fs::read_to_string(&path)?;
    let mut manifest: Value = serde_json::from_str(&before)?;
    let mut changed = false;
    for &(name, floor, target) in PAGE {
        for group in ["dependencies", "devDependencies", "peerDependencies"] {
            let Some(declared) = manifest[group][name].as_str() else {
                continue;
            };
            let installed = path
                .parent()
                .unwrap()
                .join("node_modules")
                .join(name)
                .join("package.json");
            let actual = installed
                .is_file()
                .then(|| fs::read(installed))
                .transpose()?
                .map(|v| serde_json::from_slice::<Value>(&v))
                .transpose()?;
            let observed = actual.as_ref().and_then(|v| v["version"].as_str());
            let mut row = Version {
                change: None,
                name: format!("{} {name}", path.display()),
                current: format!("{declared} (installed {})", observed.unwrap_or("unknown")),
                target: format!(">={floor}"),
            };
            let numeric = declared.trim_start_matches(['^', '~', '=']);
            if let Ok(v) = semver::Version::parse(numeric)
                && v < semver::Version::parse(floor)?
            {
                let prefix = &declared[..declared.len() - numeric.len()];
                row.change = Some(format!("{prefix}{target}"));
                manifest[group][name] = Value::String(format!("{prefix}{target}"));
                changed = true;
            }
            report.rows.push(row);
        }
    }
    if changed {
        report.edits.push(Edit {
            path,
            before,
            after: format!("{}\n", serde_json::to_string_pretty(&manifest)?),
        });
    }
    Ok(())
}
fn render_version(name: &str, mut cmd: Command, args: &[&str], target: &str) {
    let version = cmd
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()
        .filter(|r| r.status.success())
        .map(|r| String::from_utf8_lossy(&r.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unavailable".into());
    println!("{name}\t{version}\t{target}");
}
fn inspect(root: &Path) -> Result<Report> {
    let mut report = Report::default();
    cargo(&mut report, root.join("Cargo.toml"), APP)?;
    let worker = root.join("sandbox/database/Cargo.toml");
    if worker.is_file() {
        cargo(&mut report, worker, WORKER)?;
    }
    for dir in ["", "ts", "sandbox"] {
        let p = root.join(dir).join("package.json");
        if p.is_file() {
            page(&mut report, p)?;
        }
    }
    Ok(report)
}
pub fn versions(root: &Path) -> Result<()> {
    let report = inspect(root)?;
    println!("DEPENDENCY\tCURRENT\tSUPPORTED");
    println!(
        "paydirt-cli\t{}\t{}",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_VERSION")
    );
    render_version("buf", buf(), &["--version"], "1.72.0");
    render_version("cargo", Command::new("cargo"), &["--version"], "Rust 2024");
    render_version(
        "node",
        Command::new("node"),
        &["--version"],
        ">=22.12.0 for web",
    );
    for r in report.rows {
        println!("{}\t{}\t{}", r.name, r.current, r.target);
    }
    Ok(())
}

fn replace(path: &Path, contents: &str) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    temp.as_file()
        .set_permissions(fs::metadata(path)?.permissions())?;
    temp.write_all(contents.as_bytes())?;
    temp.persist(path)?;
    Ok(())
}
pub fn upgrade(root: &Path, dry: bool) -> Result<()> {
    let lock = fs::File::open(root.join("buf.yaml"))?;
    lock.lock()?;
    let report = inspect(root)?;
    if report.edits.is_empty() {
        println!("Dependencies are current.");
        return Ok(());
    }
    for edit in &report.edits {
        println!("update {}", edit.path.display());
    }
    for row in &report.rows {
        if let Some(to) = &row.change {
            println!("  {}  {} -> {to}", row.name, row.current);
        }
    }
    if dry {
        return Ok(());
    }
    for (done, edit) in report.edits.iter().enumerate() {
        if let Err(err) = replace(&edit.path, &edit.after) {
            for prior in report.edits[..done].iter().rev() {
                replace(&prior.path, &prior.before)?;
            }
            return Err(err);
        }
    }
    Ok(())
}

pub fn schema_advice(root: &Path) -> Result<()> {
    let model = paydirt_model::Model::parse(&fs::read(root.join("generated/storage/model.json"))?)?;
    let policies: Value =
        serde_json::from_slice(&fs::read(root.join("generated/storage/policies.json"))?)?;
    account_advice(root, &policies)?;
    for e in &model.entities {
        let p = &policies[&e.name];
        let opts = &p["fields"];
        let people = e.fields.iter().any(|f| opts[&f.name]["personal"] == true);
        for f in &e.fields {
            let o = &opts[&f.name];
            if f.name == e.key
                || !f.target.is_empty()
                || o.get("personal").is_some()
                || o["secret"] == true
            {
                continue;
            }
            let parts: Vec<_> = f.name.split('_').collect();
            let personal = parts.iter().any(|s| {
                matches!(
                    *s,
                    "email"
                        | "phone"
                        | "address"
                        | "birthday"
                        | "passport"
                        | "ssn"
                        | "mobile"
                        | "surname"
                        | "tel"
                        | "telephone"
                        | "fax"
                        | "birth"
                        | "birthdate"
                        | "dob"
                        | "gender"
                        | "sex"
                        | "nationality"
                        | "nickname"
                        | "forename"
                        | "resident"
                        | "portrait"
                        | "avatar"
                        | "photo"
                )
            }) || parts.windows(2).any(|p| {
                (p[1] == "name"
                    && matches!(
                        p[0],
                        "first"
                            | "last"
                            | "given"
                            | "family"
                            | "full"
                            | "middle"
                            | "real"
                            | "legal"
                    ))
                    || p == ["national", "id"]
            });
            if personal || (people && f.kind == "STRING" && f.name != "alias") {
                println!(
                    "schema option: {}.{} should declare personal: true or personal: false",
                    e.name, f.name
                );
            }
        }
        if let Some(why) = p["entity"]["trail"]["off"]["why"].as_str() {
            for f in &e.fields {
                if opts[&f.name]["secret"] == true {
                    println!(
                        "schema option: {} is kept off the trail and holds a secret ({}), reason: {why}",
                        e.name, f.name
                    );
                }
            }
        }
    }
    let config: Value = serde_json::from_slice(&fs::read(root.join("app.json"))?)?;
    if let Some(w) = config.get("watch") {
        let broker = w["broker"]
            .as_str()
            .ok_or("wiring: watch.broker must be declared")?;
        if !matches!(broker, "memory" | "postgres" | "nats" | "none") {
            return Err(format!("wiring: unknown watch.broker {broker}").into());
        }
        if w.get("drain_every_ms").is_some_and(|v| !v.is_null())
            && (w["outbox"] != true
                || broker == "none"
                || w["drain_every_ms"].as_u64().unwrap_or(0) == 0)
        {
            return Err(
                "wiring: draining requires a broker, outbox and a positive drain_every_ms".into(),
            );
        }
        if broker == "nats" && w["dsn"].as_str().unwrap_or_default().is_empty() {
            return Err("wiring: watch.dsn required for NATS".into());
        }
    }
    Ok(())
}

pub fn legacy_diagnostics(root: &Path) -> Result<()> {
    for (file, bytes) in generate::files(&root.join("proto"))? {
        if file.extension().is_none_or(|e| e != "proto") {
            continue;
        }
        let source = std::str::from_utf8(&bytes)?;
        for (n, line) in source.lines().enumerate() {
            let line = line.split("//").next().unwrap_or("");
            if line
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .any(|s| s == "impersonal")
                && line
                    .split("impersonal")
                    .nth(1)
                    .is_some_and(|s| s.trim_start().starts_with(':'))
            {
                return Err(format!("obsolete declaration in proto/{}:{}: replace impersonal: true with personal: false",file.display(),n+1).into());
            }
        }
    }
    Ok(())
}

fn account_advice(root: &Path, policies: &Value) -> Result<()> {
    use syn::visit::Visit;
    #[derive(Default)]
    struct Reads(std::collections::BTreeSet<String>);
    impl<'ast> Visit<'ast> for Reads {
        fn visit_expr_method_call(&mut self, expr: &'ast syn::ExprMethodCall) {
            if expr.method == "account_reads"
                && let Some(syn::Expr::Lit(method)) = expr.args.iter().nth(1)
                && let syn::Lit::Str(method) = &method.lit
            {
                self.0.insert(method.value());
            }
            syn::visit::visit_expr_method_call(self, expr);
        }
    }
    let mut reads = Reads::default();
    if root.join("src").is_dir() {
        for (path, bytes) in generate::files(&root.join("src"))? {
            if path.extension().is_some_and(|x| x == "rs") {
                reads.visit_file(&syn::parse_file(std::str::from_utf8(&bytes)?)?);
            }
        }
    }
    for (entity, policy) in policies.as_object().into_iter().flatten() {
        for (name, method) in policy["methods"].as_object().into_iter().flatten() {
            if method["policy"].get("account").is_none() {
                continue;
            }
            // The built-in memberships sink declares its own account-to-holder relation.
            if policy["entity"]["own"] == "OWN_HOLDER" && name == "Memberships" {
                continue;
            }
            let path = format!("/{entity}Service/{name}");
            if !reads.0.contains(&path) {
                println!(
                    "account scope: {path} has no account_reads declaration. Pass the method path and AccountReads::default() to declare an intentionally empty scope."
                );
            }
        }
    }
    Ok(())
}
