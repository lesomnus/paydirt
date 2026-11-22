//! Schema-derived command paths. Help and completion work before opening a connection.
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
    aliases: BTreeMap<String, String>,
    hidden: BTreeSet<String>,
    qualified: BTreeMap<String, String>,
    pub formats: BTreeMap<String, fn(&Value) -> Result<String>>,
    pub renderers: BTreeMap<String, fn(&Value) -> Result<String>>,
    pub columns: BTreeMap<String, Vec<(&'static str, &'static str)>>,
}
fn below(path: &str, group: &str) -> bool {
    path == group
        || path
            .strip_prefix(group)
            .is_some_and(|rest| rest.starts_with('/'))
}
fn valid(path: &str) -> bool {
    path.split('/').all(|word| {
        !word.is_empty() && !word.starts_with('-') && !word.chars().any(char::is_whitespace)
    })
}
#[allow(dead_code)]
impl Tree {
    pub fn packages(policies: &paydirt_store::Policies) -> BTreeSet<&str> {
        policies
            .iter()
            .filter(|(_, p)| !p.methods.is_empty())
            .filter_map(|(n, _)| n.rsplit_once('.').map(|(package, _)| package))
            .collect()
    }
    pub fn new(policies: &paydirt_store::Policies) -> Result<Self> {
        Self::new_in(policies, &[])
    }
    pub fn new_in(policies: &paydirt_store::Policies, packages: &[&str]) -> Result<Self> {
        let mut tree = Self {
            routes: Default::default(),
            aliases: Default::default(),
            hidden: Default::default(),
            qualified: Default::default(),
            formats: Default::default(),
            renderers: Default::default(),
            columns: Default::default(),
        };
        let entities: Vec<_> = policies
            .iter()
            .filter(|(n, p)| {
                !p.methods.is_empty()
                    && n.rsplit_once('.')
                        .is_some_and(|(pkg, _)| packages.is_empty() || packages.contains(&pkg))
            })
            .collect();
        let mut counts = BTreeMap::new();
        for (n, _) in &entities {
            *counts
                .entry(n.rsplit('.').next().unwrap().to_lowercase())
                .or_insert(0) += 1;
        }
        for (n, p) in entities {
            let qualified = n.to_lowercase();
            let word = n.rsplit('.').next().unwrap().to_lowercase();
            let path = if counts[&word] == 1 {
                word
            } else {
                qualified.clone()
            };
            for (rpc, m) in &p.methods {
                if m["client_stream"] == true {
                    continue;
                }
                let route = format!("{path}/{}", rpc.to_lowercase());
                tree.add(
                    &route,
                    Command::Rpc {
                        entity: n.clone(),
                        rpc: rpc.clone(),
                    },
                )?;
                if rpc == "List" {
                    tree.alias(&format!("{path}/ls"), &route)?;
                }
            }
            if qualified != path && tree.exists(&path) {
                tree.alias(&qualified, &path)?;
            }
            tree.qualified.insert(qualified, path);
        }
        Ok(tree)
    }
    fn canonical(&self, path: &str) -> String {
        let mut path = path.to_owned();
        // Alias destinations are registered as canonical paths. An aliased group may itself contain aliases.
        loop {
            let found = self
                .aliases
                .iter()
                .filter(|(name, _)| below(&path, name))
                .max_by_key(|(name, _)| name.len());
            let Some((name, destination)) = found else {
                return path;
            };
            path = format!("{destination}{}", &path[name.len()..]);
        }
    }
    fn exists(&self, path: &str) -> bool {
        let dest = self.canonical(path);
        self.routes.keys().any(|p| below(p, &dest))
    }
    fn available(&self, path: &str) -> Result<()> {
        if !valid(path) {
            return Err(format!("invalid command word {path}").into());
        }
        if self.exists(path) {
            return Err(format!("{path} already runs {}", self.canonical(path)).into());
        }
        let mut parents = path;
        while let Some((prefix, _)) = parents.rsplit_once('/') {
            if self.routes.contains_key(&self.canonical(prefix))
                || self.aliases.contains_key(prefix)
            {
                return Err(format!("command {prefix} already exists").into());
            }
            parents = prefix;
        }
        Ok(())
    }
    pub fn add(&mut self, path: &str, cmd: Command) -> Result<()> {
        self.available(path)?;
        self.routes.insert(path.into(), cmd);
        Ok(())
    }
    pub fn replace(&mut self, path: &str, cmd: Command) -> Result<()> {
        let key = self.canonical(path);
        *self
            .routes
            .get_mut(&key)
            .ok_or_else(|| format!("unknown command {path}"))? = cmd;
        Ok(())
    }
    pub fn alias(&mut self, alias: &str, path: &str) -> Result<()> {
        if !self.exists(path) {
            return Err(format!("nothing to alias at {path}").into());
        }
        let target = self.canonical(path);
        if self.canonical(alias) == target {
            return Ok(());
        }
        self.available(alias)?;
        if below(alias, &target) {
            return Err("an alias cannot point to its ancestor".into());
        }
        self.aliases.insert(alias.into(), target);
        Ok(())
    }
    /// Install sibling aliases together, including aliases for entity groups.
    pub fn aliases(&mut self, path: &str, words: &[&str]) -> Result<()> {
        if words.is_empty() {
            return Err("no alias to give".into());
        }
        let original = self.aliases.clone();
        for word in words {
            if word.contains('/') || !valid(word) {
                self.aliases = original;
                return Err(format!("invalid alias {word}").into());
            }
            let alias = path.rsplit_once('/').map_or_else(
                || word.to_string(),
                |(parent, _)| format!("{parent}/{word}"),
            );
            if let Err(e) = self.alias(&alias, path) {
                self.aliases = original;
                return Err(e);
            }
        }
        Ok(())
    }
    /// Register a command and its sibling words atomically. Replacements keep existing aliases.
    pub fn add_with_aliases(&mut self, path: &str, cmd: Command, words: &[&str]) -> Result<()> {
        self.add(path, cmd)?;
        if !words.is_empty()
            && let Err(e) = self.aliases(path, words)
        {
            self.routes.remove(path);
            return Err(e);
        }
        Ok(())
    }
    pub fn replace_with_aliases(&mut self, path: &str, cmd: Command, words: &[&str]) -> Result<()> {
        let key = self.canonical(path);
        if !self.routes.contains_key(&key) {
            return Err(format!("unknown command {path}").into());
        }
        if !words.is_empty() {
            self.aliases(path, words)?;
        }
        self.replace(path, cmd)
    }
    pub fn hide(&mut self, path: &str) -> Result<()> {
        if !self.exists(path) {
            return Err(format!("nothing to hide at {path}").into());
        }
        self.hidden.insert(path.into());
        Ok(())
    }
    pub fn drop_path(&mut self, path: &str) -> Result<()> {
        if !self.exists(path) {
            return Err(format!("unknown command {path}").into());
        }
        let target = self.canonical(path);
        self.routes.retain(|r, _| !below(r, &target));
        self.aliases
            .retain(|a, t| !below(t, &target) && !below(a, &target));
        Ok(())
    }
    pub fn mount(&mut self, prefix: &str, package: &str) -> Result<()> {
        let base = format!("{}.", package.to_lowercase());
        let mut pending = Vec::new();
        for (qualified, group) in &self.qualified {
            let Some(short) = qualified.strip_prefix(&base) else {
                continue;
            };
            for (p, cmd) in self.routes.iter().filter(|(p, _)| below(p, group)) {
                let path = format!("{prefix}/{short}{}", &p[group.len()..]);
                self.available(&path)?;
                pending.push((path, cmd.clone()));
            }
        }
        if pending.is_empty() {
            return Err(format!("unknown package {package}").into());
        }
        let mut mounted_aliases = Vec::new();
        for (qualified, group) in &self.qualified {
            if let Some(short) = qualified.strip_prefix(&base) {
                for (alias, target) in &self.aliases {
                    if let Some(suffix) = alias.strip_prefix(&(group.to_owned() + "/"))
                        && below(target, group)
                    {
                        let name = format!("{prefix}/{short}/{suffix}");
                        self.available(&name)?;
                        mounted_aliases
                            .push((name, format!("{prefix}/{short}{}", &target[group.len()..])));
                    }
                }
            }
        }
        self.routes.extend(pending);
        self.aliases.extend(mounted_aliases);
        Ok(())
    }
    pub fn complete(&self, path: &str) -> Vec<String> {
        let key = self.canonical(path);
        if let Some(cmd) = self.routes.get(&key) {
            let mut flags = vec!["--help", "--in", "--out", "--tz", "-o"];
            if let Command::Rpc { rpc, .. } = cmd {
                match rpc.as_str() {
                    "List" => flags.extend(["--next", "--order", "--reverse", "--size"]),
                    "Watch" => flags.push("--retry"),
                    _ => {}
                }
            }
            flags.sort_unstable();
            return flags.into_iter().map(str::to_owned).collect();
        }
        let prefix = if key.is_empty() {
            String::new()
        } else {
            format!("{key}/")
        };
        self.routes
            .keys()
            .filter(|p| !self.hidden.iter().any(|h| below(p, h)))
            .filter_map(|p| p.strip_prefix(&prefix)?.split('/').next())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    pub fn complete_words(&self, words: &[String]) -> Vec<String> {
        let path = words.join("/");
        if words.is_empty() || self.exists(&path) {
            return self.complete(&path);
        }
        let (partial, parents) = words.split_last().unwrap();
        self.complete(&parents.join("/"))
            .into_iter()
            .filter(|w| w.starts_with(partial))
            .collect()
    }
    pub fn select<'a, 'b>(
        &'a self,
        args: &'b [String],
    ) -> Result<Option<(&'a Command, &'b [String])>> {
        let mut path = String::new();
        for i in 0..=args.len() {
            let key = self.canonical(&path);
            let next = args.get(i).map(String::as_str);
            let help = matches!(next, Some("--help" | "-h"));
            if let Some(cmd) = self.routes.get(&key) {
                if !help {
                    return Ok(Some((cmd, &args[i..])));
                }
                println!(
                    "{} {}",
                    path.replace('/', " "),
                    self.complete(&path).join(" ")
                );
                return Ok(None);
            }
            if help || next.is_none() {
                let mut names = self.complete(&path);
                if path.is_empty() {
                    names.extend(["completion".into(), "config".into()]);
                    names.sort();
                }
                if names.is_empty() {
                    return Err(format!("unknown command {path}").into());
                }
                println!("{}", names.join(" "));
                return Ok(None);
            }
            if i > 0 {
                path.push('/');
            }
            path.push_str(&next.unwrap().to_lowercase());
        }
        Err(format!("unknown command {path}").into())
    }
}
