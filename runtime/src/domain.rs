//! Names assigned to the domain byte of Payday identifiers.
use crate::{Fault, Policies, Result};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default, Debug)]
pub struct Domains {
    entities: BTreeMap<String, u8>,
    former_entities: BTreeMap<String, u8>,
    words: BTreeMap<String, BTreeSet<u8>>,
    former_words: BTreeMap<String, BTreeSet<u8>>,
    names: BTreeMap<u8, BTreeSet<String>>,
}

fn only<T: Ord>(set: &BTreeSet<T>) -> Option<&T> {
    if set.len() == 1 { set.first() } else { None }
}

pub fn word(message: &str, declared: &str) -> String {
    if !declared.is_empty() {
        return declared.to_owned();
    }
    let mut out = String::new();
    for c in message.chars() {
        if c.is_ascii_uppercase() && !out.is_empty() {
            out.push('-');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

impl Domains {
    pub fn register(&mut self, entity: &str, domain: u8, declared: &str) -> Result<()> {
        self.record(entity, domain, declared, false)
    }
    pub fn formerly(&mut self, entity: &str, domain: u8, declared: &str) -> Result<()> {
        self.record(entity, domain, declared, true)
    }
    fn record(&mut self, entity: &str, domain: u8, declared: &str, former: bool) -> Result<()> {
        if entity.is_empty() || domain == 0 {
            return Err(Fault::invalid("entity and a nonzero domain are required"));
        }
        let (entities, words) = if former {
            (&mut self.former_entities, &mut self.former_words)
        } else {
            (&mut self.entities, &mut self.words)
        };
        if entities.get(entity).is_some_and(|v| *v != domain) {
            return Err(Fault::invalid(format!(
                "{entity} has conflicting domain numbers"
            )));
        }
        let name = word(entity.rsplit('.').next().unwrap_or(entity), declared);
        entities.insert(entity.to_owned(), domain);
        words.entry(name.clone()).or_default().insert(domain);
        if !former {
            self.names.entry(domain).or_default().insert(name);
        }
        Ok(())
    }
    pub fn from_policies(policies: &Policies) -> Result<Self> {
        let mut result = Self::default();
        for (entity, policy) in policies {
            let p = &policy.entity;
            let Some(d) = p["domain"].as_u64() else {
                continue;
            };
            let d = u8::try_from(d).map_err(|_| Fault::invalid("domain exceeds one byte"))?;
            result.register(entity, d, p["name"].as_str().unwrap_or_default())?;
            for v in p["formerly"].as_array().into_iter().flatten() {
                let name = v["message"]
                    .as_str()
                    .ok_or_else(|| Fault::invalid("former message required"))?;
                let qualified = entity
                    .rsplit_once('.')
                    .map(|(pkg, _)| format!("{pkg}.{name}"))
                    .unwrap_or_else(|| name.into());
                result.formerly(&qualified, d, v["name"].as_str().unwrap_or_default())?;
            }
        }
        Ok(result)
    }
    pub fn lookup(&self, entity: &str) -> Option<u8> {
        self.entities
            .get(entity)
            .or_else(|| self.former_entities.get(entity))
            .copied()
    }
    pub fn domain_of(&self, name: &str) -> Option<u8> {
        self.words
            .get(name)
            .or_else(|| self.former_words.get(name))
            .and_then(only)
            .copied()
    }
    pub fn name(&self, domain: u8) -> String {
        self.names
            .get(&domain)
            .and_then(only)
            .cloned()
            .unwrap_or_else(|| {
                if domain == 0 {
                    "unknown".into()
                } else {
                    format!("domain({domain})")
                }
            })
    }
    pub fn entries(&self) -> impl Iterator<Item = (u8, &str)> {
        self.names
            .iter()
            .filter_map(|(id, names)| Some((*id, only(names)?.as_str())))
    }
    pub fn check_id(&self, id: &[u8], want: u8) -> Result<()> {
        let text =
            paydirt::format_id(id).ok_or_else(|| Fault::invalid("id: expected sixteen bytes"))?;
        if id == [0; 16] {
            return Err(Fault::invalid(
                "id: that one names nobody, so nothing may hold it",
            ));
        }
        let got = id[9];
        if !paydirt::has_domain(id, got) {
            return Err(Fault::invalid(format!(
                "id: {text} is not an identifier this app makes"
            )));
        }
        if got != want {
            return Err(Fault::invalid(format!(
                "id: names a {}, and this is a {}",
                self.name(got),
                self.name(want)
            )));
        }
        Ok(())
    }
}
