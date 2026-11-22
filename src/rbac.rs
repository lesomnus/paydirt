//! Tenant-qualified method roles and stable, validated policy snapshots.
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

pub fn alias(s: &str) -> bool {
    s.len() <= 63
        && s.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && s.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}
fn tenant(name: &str) -> Result<&str, String> {
    let (t, subject) = name
        .strip_prefix('@')
        .and_then(|s| s.split_once('/'))
        .ok_or_else(|| format!("invalid qualified name {name}"))?;
    for part in [t, subject] {
        if !alias(part)
            && !Uuid::parse_str(part).is_ok_and(|id| !id.is_nil() && id.to_string() == part)
        {
            return Err(format!("invalid component in {name}"));
        }
    }
    Ok(t)
}
fn method_pattern(pattern: &str) -> bool {
    let Some((service, method)) = pattern.strip_prefix('/').and_then(|p| p.split_once('/')) else {
        return false;
    };
    let Some((pkg, service)) = service.rsplit_once('.') else {
        return false;
    };
    let ident = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .enumerate()
                .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
    };
    (pkg == "*" || pkg.split('.').all(ident))
        && (service == "*" || ident(service))
        && (method == "*" || ident(method))
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rules {
    pub roles: BTreeMap<String, Role>,
    pub groups: BTreeMap<String, Group>,
    pub bindings: Vec<Binding>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Role {
    pub methods: Vec<String>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Group {
    pub members: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub role: String,
    pub holder: Option<String>,
    pub group: Option<String>,
}
#[derive(Default, Clone, PartialEq, Eq)]
pub struct Snapshot {
    grants: BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
    holders: BTreeMap<String, BTreeSet<String>>,
}
impl Snapshot {
    pub fn new(rules: Rules) -> Result<Self, String> {
        let mut s = Self::default();
        for (role, r) in &rules.roles {
            tenant(role)?;
            for m in &r.methods {
                if !method_pattern(m) {
                    return Err(format!("invalid method {m} for {role}"));
                }
            }
        }
        for (name, g) in &rules.groups {
            let t = tenant(name)?;
            for h in &g.members {
                if tenant(h)? != t {
                    return Err(format!("foreign group member {h}"));
                }
            }
        }
        for b in &rules.bindings {
            let role = rules
                .roles
                .get(&b.role)
                .ok_or_else(|| format!("unknown role {}", b.role))?;
            let (subject, members) = match (&b.holder, &b.group) {
                (Some(h), None) => (h, std::slice::from_ref(h)),
                (None, Some(g)) => (
                    g,
                    rules
                        .groups
                        .get(g)
                        .ok_or_else(|| format!("unknown group {g}"))?
                        .members
                        .as_slice(),
                ),
                _ => return Err("a binding needs exactly one holder or group".into()),
            };
            if tenant(subject)? != tenant(&b.role)? {
                return Err("cross-tenant role binding".into());
            }
            for h in members {
                s.grants
                    .entry(h.clone())
                    .or_default()
                    .entry(b.role.clone())
                    .or_default()
                    .extend(role.methods.iter().cloned());
                s.holders
                    .entry(b.role.clone())
                    .or_default()
                    .insert(h.clone());
            }
        }
        Ok(s)
    }
    pub fn allows(&self, holder: &str, method: &str) -> bool {
        self.grants
            .get(holder)
            .into_iter()
            .flat_map(|rs| rs.values().flatten())
            .any(|p| crate::tenant::covers(p, method))
    }
    pub fn holders(&self, role: &str) -> Result<Vec<String>, String> {
        tenant(role)?;
        Ok(self
            .holders
            .get(role)
            .into_iter()
            .flatten()
            .cloned()
            .collect())
    }
}
#[derive(Clone)]
pub struct Policy(Arc<RwLock<(u64, Arc<Snapshot>)>>);
impl Policy {
    pub fn new(snapshot: Snapshot) -> Self {
        Self(Arc::new(RwLock::new((0, Arc::new(snapshot)))))
    }
    pub fn pin(&self) -> (u64, Arc<Snapshot>) {
        self.0.read().unwrap().clone()
    }
    pub fn replace(&self, next: Snapshot) {
        let mut p = self.0.write().unwrap();
        if *p.1 != next {
            p.0 += 1;
            p.1 = Arc::new(next);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn file(path: std::path::PathBuf, refresh: std::time::Duration) -> Result<Self, String> {
        fn load(path: &std::path::Path) -> Result<Snapshot, String> {
            let read = || std::fs::read(path).map_err(|e| e.to_string());
            let bytes = read()?;
            if read()? != bytes {
                return Err("policy file changed during read".into());
            }
            let rules = serde_yaml::from_slice(&bytes).map_err(|e| e.to_string())?;
            Snapshot::new(rules)
        }
        let policy = Self::new(load(&path)?);
        let weak = Arc::downgrade(&policy.0);
        std::thread::Builder::new()
            .name("paydirt-policy".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(refresh.max(std::time::Duration::from_millis(1)));
                    let Some(store) = weak.upgrade() else { break };
                    match load(&path) {
                        Ok(next) => Policy(store).replace(next),
                        Err(e) => eprintln!("policy reload failed for {}: {e}", path.display()),
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(policy)
    }
}
