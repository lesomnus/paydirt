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
pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

include!("../generated/storage/commands.rs");

fn reference(v: &str, entity: &str) -> Result<Value> {
    let domains = __APP_CRATE__::transport::domains();
    let short = entity.rsplit('.').next().unwrap_or(entity).to_ascii_lowercase();
    let expected = domains.lookup(entity);
    if let Ok(id) = uuid::Uuid::parse_str(v) {
        if let Some(domain) = expected {
            domains.check_id(id.as_bytes(), domain)?;
        }
        return Ok(json!({"id":STANDARD.encode(id.as_bytes())}));
    }
    let (name, suffix) = v.rsplit_once('#').unwrap_or((v, ""));
    if !suffix.is_empty() {
        let kind = suffix.to_ascii_lowercase();
        let actual = domains.domain_of(&kind);
        if expected.is_some() && actual.is_none() {
            let names = domains.entries().map(|(_, n)| n).collect::<Vec<_>>().join(", ");
            return Err(format!("unknown kind {kind}. Expected one of {names}").into());
        }
        if actual != expected || (actual.is_none() && kind != short) {
            return Err(format!("expected {short}, got {kind}").into());
        }
    }
    let name = name.strip_prefix('@').ok_or("expected UUID or @name")?;
    if name.is_empty() {
        return Err("empty row name".into());
    }
    if let Some((tenant, alias)) = name.split_once('/') {
        if tenant.is_empty() || alias.is_empty() {
            return Err("expected @tenant/alias".into());
        }
        Ok(json!({"slug":{"tenant":{"alias":tenant.trim().to_ascii_lowercase()},"alias":alias.trim().to_ascii_lowercase()}}))
    } else {
        Ok(json!({"alias":name.trim().to_ascii_lowercase()}))
    }
}
// Interpret only descriptor-declared types, including map values and repeated refs.
fn translate(
    value: &mut Value,
    ty: &str,
    schema: &Value,
    input: bool,
    zone: Option<chrono_tz::Tz>,
) -> Result<()> {
    if let Value::Array(values) = value {
        for v in values {
            translate(v, ty, schema, input, zone)?;
        }
        return Ok(());
    }
    if ty == "google.protobuf.Timestamp" {
        if let (Some(zone), Some(stamp)) = (zone, value.as_str()) {
            let date = chrono::DateTime::parse_from_rfc3339(stamp)?;
            *value = json!(
                date.with_timezone(&zone)
                    .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
            );
        }
        return Ok(());
    }
    if let Some(text) = value.as_str() {
        if input
            && let Some(entity) = ty.strip_suffix("Ref")
            && !schema[ty].is_null()
            && !schema[entity].is_null()
        {
            *value = reference(text, entity)?;
        }
        return Ok(());
    }
    let Some(fields) = schema[ty]["fields"].as_array() else {
        return Ok(());
    };
    let Some(obj) = value.as_object_mut() else {
        return Ok(());
    };
    for (k, v) in obj {
        let field = if schema[ty]["map"] == true {
            fields.iter().find(|f| f["number"] == 2)
        } else {
            fields
                .iter()
                .find(|f| f["name"] == k.as_str() || f["proto"] == k.as_str())
        };
        let Some(f) = field else { continue };
        if f["uuid"] == true {
            let convert = |v: &mut Value| {
                if let Some(text) = v.as_str() {
                    if input {
                        if let Ok(id) = uuid::Uuid::parse_str(text) {
                            *v = json!(STANDARD.encode(id.as_bytes()));
                        }
                    } else if let Ok(bytes) = STANDARD.decode(text)
                        && let Ok(id) = uuid::Uuid::from_slice(&bytes)
                    {
                        *v = json!(id.to_string());
                    }
                }
            };
            if let Some(list) = v.as_array_mut() {
                list.iter_mut().for_each(convert)
            } else {
                convert(v)
            }
        } else if let Some(target) = f["target"].as_str().filter(|s| !s.is_empty()) {
            translate(v, target, schema, input, zone)?;
        }
    }
    Ok(())
}
pub async fn invoke(
    client: &HttpClient,
    config: &ClientConfig,
    entity: &str,
    rpc: &str,
    req: Value,
) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    dispatch(client, config, entity, rpc, req, |v| {
        rows.push(v);
        Ok(())
    })
    .await?;
    Ok(rows)
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
fn print(
    mut v: Value,
    format: &str,
    message: Option<&MessageDescriptor>,
    entity: &str,
    output_ty: &str,
    schema: &Value,
    zone: chrono_tz::Tz,
    tree: &crate::tree::Tree,
) -> Result<()> {
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
        translate(
            &mut v,
            output_ty,
            schema,
            false,
            (!matches!(format, "json" | "name")).then_some(zone),
        )?;
    }
    if let Some(render) = tree.formats.get(format) {
        println!("{}", render(&v)?);
        return Ok(());
    }
    if matches!(format, "table" | "wide")
        && let Some(cols) = tree.columns.get(entity)
    {
        let rows = v["items"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(std::slice::from_ref(&v));
        println!(
            "{}",
            cols.iter()
                .map(|(title, _)| *title)
                .collect::<Vec<_>>()
                .join("\t")
        );
        for row in rows {
            println!(
                "{}",
                cols.iter()
                    .map(|(_, path)| match row.pointer(path) {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Null) | None => String::new(),
                        Some(v) => v.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join("\t")
            );
        }
        return Ok(());
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
    let mut tree = crate::tree::Tree::new(&policies)?;
    crate::commands::configure(&mut tree)?;
    let Some((target, tail)) = tree.select(args)? else {
        return Ok(());
    };
    let (entity, rpc) = match target {
        crate::tree::Command::Rpc { entity, rpc } => (entity.as_str(), rpc.as_str()),
        crate::tree::Command::Custom(handler) => {
            let (client, config) = connection()?;
            return handler(&client, &config, tail).await;
        }
    };
    let policy = &policies[entity];
    let meta = &policy.methods[rpc];
    let schema: Value = serde_json::from_str(__APP_CRATE__::transport::ADMISSION_SCHEMA)?;
    let mut body = json!({});
    let mut name = None;
    let mut document = None;
    let mut output = "pretty";
    let mut input = "json";
    let mut retry = false;
    let default_zone = env::var(paydirt::environment::APP_TZ).unwrap_or_else(|_| "UTC".into());
    let mut zone = default_zone.as_str();
    let mut args = tail.iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" | "--out" => output = args.next().ok_or("missing output format")?,
            "--tz" => zone = args.next().ok_or("missing --tz display zone")?,
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
                name = Some(reference(v, entity)?)
            }
            v if name.is_none() && !v.starts_with(['{', '-']) && v.contains('=') => {
                let (field, raw) = v.split_once('=').unwrap();
                if rpc == "Add" {
                    return Err("a unique key names an existing row".into());
                }
                let reference_type = format!("{entity}Ref");
                if !schema[&reference_type]["fields"].as_array().is_some_and(|fs|
                    fs.iter().any(|f| f["name"] == field || f["proto"] == field)) {
                    return Err(format!("unknown reference key {field}").into());
                }
                let val = serde_json::from_str(raw).unwrap_or_else(|_| json!(raw));
                name = Some(json!({field:val}));
            }
            v if document.is_none() && (v == "-" || !v.starts_with('-')) => document = Some(v),
            _ => return Err(format!("unexpected argument {a}").into()),
        }
    }
    let zone = zone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| format!("unknown --tz display zone: {zone}"))?;
    if !matches!(input, "json" | "protojson" | "prototext")
        || (!output.starts_with("template=")
            && !tree.formats.contains_key(output)
            && !matches!(
                output,
                "pretty" | "json" | "protojson" | "name" | "table" | "wide" | "prototext"
            ))
    {
        return Err("unsupported input or output format".into());
    }
    if let Some(ref key) = name {
        match rpc {
            "Get" | "Patch" | "Apply" => body["ref"] = key.clone(),
            "Erase" => body = key.clone(),
            "List" | "Watch" => body["filters"] = json!([{"ref":key}]),
            "Add" => body = key.get("slug").unwrap_or(key).clone(),
            _ if meta["input"].as_str().is_some_and(|ty| {
                schema[ty.trim_start_matches('.')]["fields"].as_array().is_some_and(|fs|
                    fs.iter().any(|f| f["name"] == "ref" && f["target"] == format!("{entity}Ref")))
            }) => body["ref"] = key.clone(),
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
        translate(
            &mut body,
            meta["input"].as_str().unwrap().trim_start_matches('.'),
            &schema,
            true,
            None,
        )?;
    }
    let (client, config) = connection()?;
    loop {
        match dispatch(&client, &config, entity, rpc, body.clone(), |v| {
            print(
                v,
                output,
                response.as_ref(),
                entity,
                meta["output"].as_str().unwrap().trim_start_matches('.'),
                &schema,
                zone,
                &tree,
            )
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

pub(crate) fn rpc_client(
    endpoint: &str,
    token: Option<&str>,
) -> Result<(HttpClient, ClientConfig)> {
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
        _ => return Err("endpoint must be an HTTP or HTTPS URL".into()),
    };
    let mut config = ClientConfig::new(uri);
    if let Some(token) = token {
        let value = http::HeaderValue::from_str(&format!("Bearer {token}"))?;
        config = config.with_default_header("authorization", value);
    }
    Ok((client, config))
}

fn connection() -> Result<(HttpClient, ClientConfig)> {
    let endpoint = env::var(paydirt::environment::PAYDIRT_URL).unwrap_or_else(|_| "http://127.0.0.1:8080".into());
    let (client, config) = rpc_client(&endpoint, env::var(paydirt::environment::PAYDIRT_TOKEN).ok().as_deref())?;
    Ok((client, config))
}
