use __APP_CRATE__::{proto, transport::POLICIES};
use base64::{Engine, engine::general_purpose::STANDARD};
use buffa_reflect::{DescriptorPool, DynamicMessage, FormatOptions, MessageDescriptor};
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
fn template_value(value: Value) -> gtmpl::Value {
    match value {
        Value::Null => gtmpl::Value::Nil,
        Value::Bool(v) => v.into(),
        Value::Number(v) => {
            if let Some(n) = v.as_i64() {
                n.into()
            } else if let Some(n) = v.as_u64() {
                n.into()
            } else {
                v.as_f64().unwrap().into()
            }
        }
        Value::String(v) => v.into(),
        Value::Array(v) => gtmpl::Value::Array(v.into_iter().map(template_value).collect()),
        Value::Object(v) => {
            gtmpl::Value::Map(v.into_iter().map(|(k, v)| (k, template_value(v))).collect())
        }
    }
}
fn print(mut v: Value, format: &str, message: Option<&MessageDescriptor>) -> Result<()> {
    if let Some(desc) = message {
        let msg = DynamicMessage::deserialize(desc.clone(), v)?;
        println!(
            "{}",
            msg.to_text_format_with_options(&FormatOptions::new().pretty(true))
                .trim_end()
        );
        return Ok(());
    }
    if format != "protojson" {
        translate(&mut v, false);
    }
    if let Some(template) = format.strip_prefix("template=") {
        println!("{}", gtmpl::template(template, template_value(v))?);
        return Ok(());
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
    let verbs = || {
        policy
            .methods
            .iter()
            .filter(|(_, m)| m["client_stream"] != true)
    };
    if args.len() == 1 || args.iter().any(|a| matches!(a.as_str(), "--help" | "-h")) {
        let names = verbs()
            .map(|(v, _)| {
                if v == "List" {
                    "list,ls".into()
                } else {
                    v.to_lowercase()
                }
            })
            .collect::<Vec<_>>();
        println!("{entity}: {}", names.join(" "));
        return Ok(());
    }
    let verb = if args[1] == "ls" { "list" } else { &args[1] };
    let (rpc, meta) = verbs()
        .find(|(name, _)| name.eq_ignore_ascii_case(verb))
        .ok_or("verb is not declared for this entity")?;
    let rpc = rpc.as_str();
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
            v if name.is_none() && (v.starts_with('@') || uuid::Uuid::parse_str(v).is_ok()) => {
                name = Some(reference(v, short)?)
            }
            v if document.is_none() && (v == "-" || !v.starts_with('-')) => document = Some(v),
            _ => return Err(format!("unexpected argument {a}").into()),
        }
    }
    if !matches!(input, "json" | "protojson" | "prototext")
        || (!output.starts_with("template=")
            && !matches!(
                output,
                "pretty" | "json" | "protojson" | "name" | "table" | "wide" | "prototext"
            ))
    {
        return Err("unsupported input or output format".into());
    }
    if let Some(ref key) = name {
        match rpc {
            "Get" | "Patch" => body["ref"] = key.clone(),
            "Erase" => body = key.clone(),
            "List" | "Watch" => body["filters"] = json!([{"ref":key}]),
            "Add" => body = key.get("slug").unwrap_or(key).clone(),
            _ => return Err("pass a request body for this verb".into()),
        }
    }
    let pool = if input == "prototext" || output == "prototext" {
        Some(DescriptorPool::decode(include_bytes!(
            "../generated/storage/descriptors.binpb"
        ))?)
    } else {
        None
    };
    let descriptor = |key: &str| -> Result<MessageDescriptor> {
        let name = meta[key]
            .as_str()
            .ok_or("missing method type")?
            .trim_start_matches('.');
        pool.as_ref()
            .and_then(|p| p.get_message_by_name(name))
            .ok_or_else(|| format!("missing descriptor for {name}").into())
    };
    let response = (output == "prototext")
        .then(|| descriptor("output"))
        .transpose()?;
    if let Some(text) = document {
        let mut data = String::new();
        let text = if text == "-" {
            io::stdin().read_to_string(&mut data)?;
            &data
        } else {
            text
        };
        let obj: Value = if input == "prototext" {
            serde_json::to_value(DynamicMessage::parse_text_format(
                descriptor("input")?,
                text,
            )?)?
        } else {
            serde_json::from_str(text)?
        };
        for (k, v) in obj.as_object().ok_or("expected a request object")? {
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
            print(v, output, response.as_ref())
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
