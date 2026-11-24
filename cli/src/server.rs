//! Application server composition depends on contracts, never on their storage.
use crate::Result;
use heck::ToSnakeCase;
use serde_json::Value;
use std::{collections::BTreeMap, fmt::Write, fs, path::Path};

pub(crate) fn generate(work: &Path) -> Result<()> {
    let policies: BTreeMap<String, Value> =
        serde_json::from_slice(&fs::read(work.join("generated/storage/policies.json"))?)?;
    let mut services: BTreeMap<String, String> = policies
        .into_iter()
        .filter(|(_, policy)| policy["methods"].as_object().is_some_and(|m| !m.is_empty()))
        .map(|(name, _)| {
            (
                format!("{name}Service"),
                name.replace('.', "__").to_snake_case(),
            )
        })
        .collect();
    let descriptor: Value = serde_json::from_slice(&fs::read(work.join("resolved.json"))?)?;
    for file in descriptor["file"].as_array().ok_or("missing files")? {
        let package = file["package"].as_str().unwrap_or_default();
        for message in file["messageType"].as_array().into_iter().flatten() {
            if message["options"].get("[payday.resource]").is_some() {
                let short = message["name"].as_str().ok_or("missing resource name")?;
                let name = format!("{package}.{short}");
                services.insert(
                    format!("{name}Service"),
                    name.replace('.', "__").to_snake_case(),
                );
            }
        }
    }
    let mut methods = String::new();
    let mut layers = String::new();
    let mut forwards = String::new();
    let mut shared = String::new();
    let mut registrations = String::new();
    for (name, accessor) in services {
        let path = name
            .split('.')
            .map(|s| match s {
                "self" | "super" | "Self" | "crate" => format!("{s}_"),
                _ => format!("r#{s}"),
            })
            .collect::<Vec<_>>()
            .join("::");
        let svc = format!("crate::services::{path}");
        writeln!(
            methods,
            "fn {accessor}(&self) -> impl {svc} {{ {svc}Unimplemented }}"
        )?;
        writeln!(
            layers,
            "fn {accessor}(&self, next: &N) -> impl {svc} {{ next.{accessor}() }}"
        )?;
        writeln!(
            forwards,
            "fn {accessor}(&self) -> impl {svc} {{ self.layer.{accessor}(&self.next) }}"
        )?;
        writeln!(
            shared,
            "fn {accessor}(&self) -> impl {svc} {{ self.as_ref().{accessor}() }}"
        )?;
        writeln!(
            registrations,
            "if include({name:?}) {{ router = {svc}Ext::register(::std::sync::Arc::new(server.{accessor}()), router); }}"
        )?;
    }
    let source = format!(
        r#"
        /// Services supplied by an application, independent of any database.
        /// An omitted service answers Unimplemented.
        pub trait Server {{
            {methods}
            /// Add an outer layer. The last layer added handles a request first.
            fn layer<L: Layer<Self>>(self, layer: L) -> Overlay<Self, L>
            where Self: Sized {{ Overlay {{ next: self, layer }} }}
        }}
        #[derive(Clone, Copy, Default)]
        pub struct Unimplemented;
        impl Server for Unimplemented {{}}
        /// Each accessor may replace a service or wrap the service behind it.
        pub trait Layer<N: Server + ?Sized> {{ {layers} }}
        #[derive(Clone)]
        pub struct Overlay<N, L> {{ pub next: N, pub layer: L }}
        impl<N: Server, L: Layer<N>> Server for Overlay<N, L> {{ {forwards} }}
        impl<T: Server + ?Sized> Server for ::std::sync::Arc<T> {{ {shared} }}
        pub fn mount(router: ::connectrpc::Router, server: &impl Server) -> ::connectrpc::Router {{
            mount_if(router, server, |_| true)
        }}
        /// Select services at registration; this does not authorize requests.
        pub fn mount_if(mut router: ::connectrpc::Router, server: &impl Server,
            include: impl Fn(&str) -> bool) -> ::connectrpc::Router {{
            {registrations}
            router
        }}
    "#
    );
    fs::write(
        work.join("generated/server.rs"),
        prettyplease::unparse(&syn::parse_file(&source)?),
    )?;
    Ok(())
}
