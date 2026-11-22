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
