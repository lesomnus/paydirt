use __APP_CRATE__::{proto, transport::POLICIES};
use base64::{Engine, engine::general_purpose::STANDARD};
use connectrpc::client::{ClientConfig, HttpClient};
use serde_json::{Value, json};
use std::{
    env,
    error::Error,
    io::{self, Read, Write},
    sync::Arc,
};
type Result<T> = std::result::Result<T, Box<dyn Error>>;

include!("../generated/storage/commands.rs");

fn reference(v: &str, entity: &str) -> Result<Value> {
    if let Ok(id) = uuid::Uuid::parse_str(v) {
        return Ok(json!({"id":STANDARD.encode(id.as_bytes())}));
    }
    let (name, kind) = v.rsplit_once('#').unwrap_or((v, ""));
    if !kind.is_empty() && !kind.eq_ignore_ascii_case(entity) {
        return Err(format!("expected {entity}, got {kind}").into());
    }
    let name = name.strip_prefix('@').ok_or("expected UUID or @name")?;
    if name.is_empty() {
        return Err("empty row name".into());
    }
    if let Some((tenant, alias)) = name.split_once('/') {
        if tenant.is_empty() || alias.is_empty() {
            return Err("expected @tenant/alias".into());
        }
        Ok(json!({"slug":{"tenant":{"alias":tenant},"alias":alias}}))
    } else {
        Ok(json!({"alias":name}))
    }
}
fn translate(v: &mut Value, from_uuid: bool) {
    match v {
        Value::Array(a) => a.iter_mut().for_each(|v| translate(v, from_uuid)),
        Value::Object(o) => {
            for (k, v) in o {
                if (k == "id" || k.ends_with("Id") || k.ends_with("_id")) && v.is_string() {
                    let s = v.as_str().unwrap();
                    if from_uuid {
                        if let Ok(id) = uuid::Uuid::parse_str(s) {
                            *v = json!(STANDARD.encode(id.as_bytes()));
                        }
                    } else if let Ok(b) = STANDARD.decode(s)
                        && let Ok(id) = uuid::Uuid::from_slice(&b)
                    {
                        *v = json!(id.to_string());
                    }
                } else {
                    translate(v, from_uuid);
                }
            }
        }
        _ => {}
    }
}
fn argument(arg: &str) -> Result<Value> {
    if arg == "-" {
        let mut data = String::new();
        io::stdin().read_to_string(&mut data)?;
        Ok(serde_json::from_str(&data)?)
    } else {
        Ok(serde_json::from_str(arg)?)
    }
}
fn print(mut v: Value, format: &str) -> Result<()> {
    if format != "protojson" {
        translate(&mut v, false);
    }
    let mut out = io::stdout().lock();
    let rows = v
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| vec![v.clone()]);
    match format {
        "name" => {
            for r in rows {
                if let Some(id) = r["id"].as_str() {
                    writeln!(out, "{id}")?;
                }
            }
        }
        "json" | "protojson" => writeln!(out, "{}", serde_json::to_string(&v)?)?,
        "pretty" => writeln!(out, "{}", serde_json::to_string_pretty(&v)?)?,
        "table" | "wide" => {
            let columns: std::collections::BTreeSet<_> = rows
                .iter()
                .filter_map(Value::as_object)
                .flatten()
                .map(|(k, _)| k.as_str())
                .filter(|k| format == "wide" || *k != "id")
                .collect();
            writeln!(
                out,
                "{}",
                columns.iter().copied().collect::<Vec<_>>().join("\t")
            )?;
            for row in &rows {
                writeln!(
                    out,
                    "{}",
                    columns
                        .iter()
                        .map(|k| match &row[*k] {
                            Value::Null => String::new(),
                            Value::String(s) => s.clone(),
                            v => v.to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join("\t")
                )?;
            }
            if let Some(next) = v["next"].as_str().filter(|s| !s.is_empty()) {
                writeln!(out, "next\t{next}")?;
            }
        }
        _ => return Err(format!("unknown output format {format}").into()),
    }
    out.flush()?;
    Ok(())
}
fn method(s: &str) -> Option<&'static str> {
    Some(match s {
        "get" => "Get",
        "ls" | "list" => "List",
        "watch" => "Watch",
        "add" => "Add",
        "patch" => "Patch",
        "erase" => "Erase",
        _ => return None,
    })
}
pub async fn run(args: &[String]) -> Result<()> {
    let policies: paydirt_store::Policies = serde_json::from_str(POLICIES)?;
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        println!(
            "Usage: APP ENTITY VERB [REF] [JSON|-]\nEntities: {}",
            policies
                .iter()
                .filter(|(_, p)| !p.methods.is_empty())
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Ok(());
    }
    let mut names = policies.iter().filter(|(name, _)| {
        name.eq_ignore_ascii_case(&args[0])
            || name
                .rsplit('.')
                .next()
                .unwrap()
                .eq_ignore_ascii_case(&args[0])
    });
    let (entity, policy) = names.next().ok_or("unknown entity")?;
    if names.next().is_some() {
        return Err("use the qualified entity name".into());
    }
    if args.len() == 1 || args.iter().any(|a| matches!(a.as_str(), "--help" | "-h")) {
        println!(
            "{entity}: {}",
            ["get", "ls", "watch", "add", "patch", "erase"]
                .into_iter()
                .filter(|v| policy.methods.contains_key(method(v).unwrap()))
                .collect::<Vec<_>>()
                .join(" ")
        );
        return Ok(());
    }
    let rpc = method(&args[1])
        .filter(|m| policy.methods.contains_key(*m))
        .ok_or("verb is not declared for this entity")?;
    let short = entity.rsplit('.').next().unwrap();
    let mut body = json!({});
    let mut name = None;
    let mut document = None;
    let mut output = "pretty";
    let mut input = "json";
    let mut retry = false;
    let mut args = args[2..].iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" | "--out" => output = args.next().ok_or("missing output format")?,
            "--in" => input = args.next().ok_or("missing input format")?,
            "--retry" if rpc == "Watch" => retry = true,
            "--size" if rpc == "List" => {
                body["size"] = json!(args.next().ok_or("missing size")?.parse::<i32>()?)
            }
            "--next" if rpc == "List" => {
                body["after"] = json!(args.next().ok_or("missing cursor")?)
            }
            "--reverse" if rpc == "List" => body["reverse"] = json!(true),
            "--order" if rpc == "List" => {
                let name = args.next().ok_or("missing order")?;
                let orders = policy.entity["list"]["orders"]
                    .as_array()
                    .ok_or("no named orders")?;
                let index = orders
                    .iter()
                    .position(|o| {
                        o["name"]
                            .as_str()
                            .is_some_and(|v| v.eq_ignore_ascii_case(name))
                    })
                    .ok_or("unknown order")?;
                body["order"] = json!(index + 1);
            }
            v if (v == "-" || v.trim_start().starts_with('{')) && document.is_none() => {
                document = Some(argument(v)?)
            }
            v if !v.starts_with('-') && name.is_none() => name = Some(reference(v, short)?),
            _ => return Err(format!("unexpected argument {a}").into()),
        }
    }
    if !matches!(input, "json" | "protojson")
        || !matches!(
            output,
            "pretty" | "json" | "protojson" | "name" | "table" | "wide"
        )
    {
        return Err("unsupported input or output format".into());
    }
    if let Some(ref key) = name {
        match rpc {
            "Get" | "Patch" => body["ref"] = key.clone(),
            "Erase" => body = key.clone(),
            "List" | "Watch" => body["filters"] = json!([{"ref":key}]),
            "Add" => body = key.get("slug").unwrap_or(key).clone(),
            _ => unreachable!(),
        }
    }
    if let Some(obj) = document {
        for (k, v) in obj.as_object().ok_or("expected a JSON request object")? {
            if body.get(k).is_some() {
                return Err(format!("request field {k} specified twice").into());
            }
            body[k] = v.clone();
        }
    }
    if input == "json" {
        translate(&mut body, true);
    }
    let endpoint = env::var("PAYDIRT_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into());
    let uri: http::Uri = endpoint.parse()?;
    let client = match uri.scheme_str() {
        Some("http") => HttpClient::plaintext(),
        Some("https") => {
            let roots =
                rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::aws_lc_rs::default_provider(),
            ))
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots)
            .with_no_client_auth();
            HttpClient::with_tls(Arc::new(tls))
        }
        _ => return Err("PAYDIRT_URL must be an HTTP or HTTPS URL".into()),
    };
    let mut config = ClientConfig::new(uri);
    if let Ok(token) = env::var("PAYDIRT_TOKEN") {
        let value = http::HeaderValue::from_str(&format!("Bearer {token}"))?;
        config = config.with_default_header("authorization", value);
    }
    loop {
        match dispatch(&client, &config, entity, rpc, body.clone(), |v| {
            print(v, output)
        })
        .await
        {
            Ok(()) if rpc == "Watch" && !retry => return Err("watch stream ended".into()),
            Ok(()) if !retry => return Ok(()),
            Err(e)
                if !retry
                    || !e
                        .downcast_ref::<connectrpc::ConnectError>()
                        .is_some_and(|e| {
                            matches!(
                                e.code,
                                connectrpc::ErrorCode::Unavailable
                                    | connectrpc::ErrorCode::DeadlineExceeded
                            )
                        }) =>
            {
                return Err(e);
            }
            _ => {
                eprintln!("Watch disconnected. Reconnecting with a fresh snapshot.");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
}
