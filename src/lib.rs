//! Core types shared by native and WebAssembly Payday applications.
pub mod page;
pub mod rbac;
pub mod tenant;

use uuid::Uuid;

/// A time-ordered Payday UUIDv8. The low twelve sequence bits come from UUIDv7.
pub fn mint(domain: u8) -> [u8; 16] {
    let mut bytes = Uuid::now_v7().into_bytes();
    bytes[6] = 0x80 | (bytes[6] & 15);
    bytes[9] = domain;
    bytes
}

pub fn format_id(bytes: &[u8]) -> Option<String> {
    Uuid::from_slice(bytes).ok().map(|u| u.to_string())
}

pub fn parse_id(v: &str) -> Option<[u8; 16]> {
    Uuid::parse_str(v).ok().map(|u| u.into_bytes())
}

pub fn has_domain(bytes: &[u8], domain: u8) -> bool {
    bytes.len() == 16 && bytes[6] >> 4 == 8 && bytes[8] >> 6 == 2 && bytes[9] == domain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_go_ids_and_checks_shape() {
        // Layout example from Payday's pdid package.
        let id = Uuid::parse_str("0199c3f4-2a10-8abc-8a03-9f2e1c4d5b6a")
            .unwrap()
            .into_bytes();
        assert!(has_domain(&id, 3));
        assert!(!has_domain(&id, 7));
        assert!(!has_domain(&id[..15], 3));
        let mut wrong = id;
        wrong[8] = 0xc0;
        assert!(!has_domain(&wrong, 3));
        wrong = id;
        wrong[6] = 0x7a;
        assert!(!has_domain(&wrong, 3));
        let fresh = mint(7);
        assert!(has_domain(&fresh, 7));
        assert_ne!(fresh, mint(7));
    }

    #[test]
    fn authenticates_only_provisioned_tenant_tokens() {
        use std::collections::HashMap;
        let id = mint(1);
        let tokens =
            tenant::TenantTokens::new(HashMap::from([("first".into(), id.to_vec())]), 1).unwrap();
        assert_eq!(tokens.resolve("bearer first"), Some(id.as_slice()));
        assert_eq!(tokens.resolve("Bearer second"), None);
        assert_eq!(tokens.resolve("Basic first"), None);
        assert!(
            tenant::TenantTokens::new(HashMap::from([("first".into(), mint(7).to_vec())]), 1)
                .is_err()
        );
    }
}

pub mod watch;
