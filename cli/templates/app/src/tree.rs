//! A generated application may add and replace commands without opening the connection used for help.
use crate::operator::Result;
use connectrpc::client::{ClientConfig, HttpClient};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
};
pub type Handler = for<'a> fn(
    &'a HttpClient,
    &'a ClientConfig,
    &'a [String],
) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>>;
#[derive(Clone)]
pub enum Command {
    Rpc { entity: String, rpc: String },
    Custom(Handler),
}
pub struct Tree {
    routes: BTreeMap<String, Command>,
    hidden: BTreeSet<String>,
    pub formats: BTreeMap<String, fn(&Value) -> Result<String>>,
    pub columns: BTreeMap<String, Vec<(&'static str, &'static str)>>,
}
#[allow(dead_code)]
impl Tree {
    pub fn new(policies: &paydirt_store::Policies) -> Result<Self> {
        let mut tree = Self {
            routes: Default::default(),
            hidden: Default::default(),
            formats: Default::default(),
            columns: Default::default(),
        };
        let mut short = BTreeMap::<_, usize>::new();
        for (n, p) in policies {
            if !p.methods.is_empty() {
                *short
                    .entry(n.rsplit('.').next().unwrap().to_lowercase())
                    .or_default() += 1
            }
        }
        for (n, p) in policies {
            let word = n.rsplit('.').next().unwrap().to_lowercase();
            for (rpc, m) in &p.methods {
                if m["client_stream"] == true {
                    continue;
                }
                let cmd = Command::Rpc {
                    entity: n.clone(),
                    rpc: rpc.clone(),
                };
                let mut paths = vec![n.to_lowercase()];
                if short[&word] == 1 && paths[0] != word {
                    paths.push(word.clone());
                }
                for prefix in paths {
                    tree.add(&format!("{prefix}/{}", rpc.to_lowercase()), cmd.clone())?;
                    if rpc == "List" {
                        tree.add(&format!("{prefix}/ls"), cmd.clone())?;
                    }
                }
            }
        }
        Ok(tree)
    }
    pub fn add(&mut self, path: &str, cmd: Command) -> Result<()> {
        if path.is_empty()
            || path.split('/').any(|s| s.is_empty() || s.starts_with('-'))
            || self.routes.contains_key(path)
        {
            return Err(format!("invalid or duplicate command {path}").into());
        }
        self.routes.insert(path.into(), cmd);
        Ok(())
    }
    pub fn replace(&mut self, path: &str, cmd: Command) -> Result<()> {
        *self
            .routes
            .get_mut(path)
            .ok_or_else(|| format!("unknown command {path}"))? = cmd;
        Ok(())
    }
    pub fn alias(&mut self, alias: &str, path: &str) -> Result<()> {
        let cmd = self
            .routes
            .get(path)
            .ok_or_else(|| format!("unknown command {path}"))?
            .clone();
        self.add(alias, cmd)
    }
    pub fn hide(&mut self, path: &str) -> Result<()> {
        if !self.routes.contains_key(path) {
            return Err(format!("unknown command {path}").into());
        }
        self.hidden.insert(path.into());
        Ok(())
    }
    pub fn mount(&mut self, prefix: &str, package: &str) -> Result<()> {
        let package = format!("{}.", package.to_lowercase());
        let routes = self
            .routes
            .iter()
            .filter_map(|(p, c)| {
                p.strip_prefix(&package)
                    .map(|s| (format!("{prefix}/{s}"), c.clone()))
            })
            .collect::<Vec<_>>();
        if routes.is_empty() {
            return Err(format!("unknown package {package}").into());
        }
        for (p, c) in routes {
            self.add(&p, c)?;
        }
        Ok(())
    }
    pub fn select<'a, 'b>(
        &'a self,
        args: &'b [String],
    ) -> Result<Option<(&'a Command, &'b [String])>> {
        let mut prefix = String::new();
        for i in 0..=args.len() {
            let help = matches!(
                args.get(i).map(String::as_str),
                None | Some("--help" | "-h")
            );
            if let Some(c) = self.routes.get(&prefix) {
                if !matches!(args.get(i).map(String::as_str), Some("--help" | "-h")) {
                    return Ok(Some((c, &args[i..])));
                }
                println!("{}", prefix.replace('/', " "));
                return Ok(None);
            }
            let dot = if prefix.is_empty() {
                String::new()
            } else {
                format!("{prefix}/")
            };
            if help {
                let mut names = self
                    .routes
                    .keys()
                    .filter(|p| !self.hidden.contains(*p))
                    .filter_map(|p| p.strip_prefix(&dot)?.split('/').next())
                    .collect::<BTreeSet<_>>();
                if prefix.is_empty() { names.insert("config"); }
                if names.is_empty() {
                    return Err(format!("unknown command {prefix}").into());
                }
                println!("{}", names.into_iter().collect::<Vec<_>>().join(" "));
                return Ok(None);
            }
            prefix = dot + &args[i].to_lowercase();
        }
        Err(format!("unknown command {prefix}").into())
    }
}
