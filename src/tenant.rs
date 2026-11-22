//! Resolve a caller's opaque bearer token to a single host-provisioned tenant.
use std::collections::HashMap;

#[derive(Default)]
pub struct TenantTokens {
    principals: HashMap<String, Vec<u8>>,
}

impl TenantTokens {
    pub fn new(
        principals: HashMap<String, Vec<u8>>,
        tenant_domain: u8,
    ) -> Result<Self, &'static str> {
        if principals
            .iter()
            .any(|(token, id)| token.is_empty() || !crate::has_domain(id, tenant_domain))
        {
            return Err("invalid bearer token or tenant ID");
        }
        Ok(Self { principals })
    }

    pub fn resolve(&self, authorization: &str) -> Option<&[u8]> {
        let (scheme, token) = authorization.split_once(' ')?;
        if !scheme.eq_ignore_ascii_case("bearer") {
            return None;
        }
        self.principals.get(token).map(Vec::as_slice)
    }
}

/// Match the three whole components of a Payday action grant, including wildcards.
pub fn covers(held: &str, want: &str) -> bool {
    fn parts(s: &str) -> Option<[&str; 3]> {
        let (service, method) = s.strip_prefix('/')?.split_once('/')?;
        let (package, service) = service.rsplit_once('.')?;
        if package.is_empty() || service.is_empty() || method.is_empty() || method.contains('/') {
            return None;
        }
        Some([package, service, method])
    }
    held == want
        || parts(held)
            .zip(parts(want))
            .is_some_and(|(a, b)| a.iter().zip(b).all(|(h, w)| *h == "*" || *h == w))
}
