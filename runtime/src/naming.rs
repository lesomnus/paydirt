//! Creation-time naming policy for entities with a Payday alias.
use crate::{Fault, Frame, Result};
use serde_json::Value;

pub struct NameRequest<'a> {
    pub entity: &'a str,
    pub given: &'a str,
    pub request: &'a Value,
    pub frame: Option<&'a Frame>,
}

pub trait Namer: Send + Sync {
    fn name(&self, request: NameRequest<'_>) -> Result<String>;
}

impl<F> Namer for F
where
    F: Fn(NameRequest<'_>) -> Result<String> + Send + Sync,
{
    fn name(&self, request: NameRequest<'_>) -> Result<String> {
        self(request)
    }
}

pub fn required(request: NameRequest<'_>) -> Result<String> {
    parse(request.given)
}

pub(crate) fn parse(given: &str) -> Result<String> {
    let folded = given.trim().to_ascii_lowercase();
    if paydirt::rbac::alias(&folded) {
        Ok(folded)
    } else {
        Err(Fault::field(
            "alias",
            "expected a lowercase name beginning with a letter, with words separated by single hyphens",
        ))
    }
}

pub fn names(request: NameRequest<'_>) -> Result<String> {
    if !request.given.is_empty() {
        return parse(request.given);
    }
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz";
    let mut alias = String::with_capacity(7);
    while alias.len() < 7 {
        let mut random = [0; 8];
        getrandom::fill(&mut random).map_err(|err| Fault::internal(err.to_string()))?;
        for byte in random
            .into_iter()
            .map(usize::from)
            .filter(|n| *n < 256 - 256 % ALPHABET.len())
        {
            alias.push(ALPHABET[byte % ALPHABET.len()] as char);
            if alias.len() == 7 {
                break;
            }
        }
    }
    Ok(alias)
}
