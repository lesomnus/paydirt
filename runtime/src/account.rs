use crate::{
    Fault, Result, Transaction,
    driver::{Bind, Query},
    value,
};
use paydirt_model::{Entity, quote};
use serde_json::{Value, json};

impl Transaction<'_> {
    fn account_entities(&self) -> Result<(&Entity, &Entity, &str)> {
        let (holder, p) = self
            .policies
            .iter()
            .find(|(_, p)| p.entity["own"] == "OWN_HOLDER")
            .ok_or_else(|| Fault::precondition("no Holder schema"))?;
        let account = &p.application["accounts"]["field"];
        let field = account["name"]
            .as_str()
            .ok_or_else(|| Fault::precondition("application has no account field"))?;
        let h = self.model.entity(holder).map_err(Fault::invalid)?;
        let f = h.field(field).map_err(Fault::invalid)?;
        if Some(f.number as u64) != account["number"].as_u64() || f.kind != "UUID" {
            return Err(Fault::precondition(
                "account declaration disagrees with Holder",
            ));
        }
        let t = self
            .model
            .entity(&h.field("tenant").map_err(Fault::invalid)?.target)
            .map_err(Fault::invalid)?;
        Ok((h, t, field))
    }
    /// The identity provider supplies the account ID. Only live Holder and Tenant rows
    /// can establish a membership. Always join on the declared field and real tenant ID.
    pub(crate) fn memberships(
        &mut self,
        account: &[u8],
        bound: Option<&[u8]>,
        selector: &str,
        limit: bool,
    ) -> Result<Vec<Value>> {
        if account.len() != 16 || account.iter().all(|&b| b == 0) {
            return Err(Fault::unauthenticated("invalid account ID"));
        }
        let d = self.db.dialect();
        let (h, t, account_field) = self.account_entities()?;
        let (h, t, account_field) = (
            h.clone(),
            t.clone(),
            h.field(account_field).map_err(Fault::invalid)?.column(),
        );
        let mut q = Query::new("");
        let p = q.param(Bind::Bytes(account.to_vec()), d);
        let tenant = h.field("tenant").map_err(Fault::invalid)?;
        let key = t.key().map_err(Fault::invalid)?;
        let mut predicates = vec![format!("r.{}={p}", quote(&account_field))];
        for (entity, alias) in [(&h, "r"), (&t, "t")] {
            if let Some(f) = entity.erased() {
                predicates.push(format!("{alias}.{} IS NULL", quote(&f.column())))
            }
        }
        if let Some(id) = bound {
            let param = q.param(Bind::Bytes(id.to_vec()), d);
            predicates.push(format!("t.{}={param}", quote(&key.column())));
        }
        let selector = selector.trim();
        if !selector.is_empty() {
            let (column, bind) = if let Some(id) = paydirt::parse_id(selector) {
                if id == [0; 16] {
                    return Err(Fault::invalid("nil tenant selector"));
                }
                (key, Bind::Bytes(id.to_vec()))
            } else {
                let alias = selector
                    .strip_prefix('@')
                    .unwrap_or(selector)
                    .trim()
                    .to_ascii_lowercase();
                if !paydirt::rbac::alias(&alias) {
                    return Err(Fault::invalid("invalid tenant selector"));
                }
                (t.field("alias").map_err(Fault::invalid)?, Bind::Text(alias))
            };
            let param = q.param(bind, d);
            predicates.push(format!("t.{}={param}", quote(&column.column())));
        }
        q.sql = format!(
            "SELECT {} AS value FROM {} r JOIN {} t ON r.{}=t.{} WHERE {} ORDER BY r.{},r.{} {}",
            self.project_sql(&h)?,
            quote(&h.table()),
            quote(&t.table()),
            quote(&tenant.column()),
            quote(&key.column()),
            predicates.join(" AND "),
            quote(&h.field("date_created").map_err(Fault::invalid)?.column()),
            quote(&h.key().map_err(Fault::invalid)?.column()),
            if limit { "LIMIT 2" } else { "" }
        );
        let mut rows = Vec::new();
        for row in self.db.rows(q)? {
            let mut holder = self.decode_row(&h, &row)?;
            // Reading joined tenants is authorized by the membership query alone.
            let tenant_row = self.lookup(&t, &holder[&tenant.json], false)?;
            holder[&tenant.json] = tenant_row;
            self.strip_secrets(&t, &mut holder[&tenant.json]);
            self.strip_secrets(&h, &mut holder);
            rows.push(holder);
        }
        Ok(rows)
    }
    fn strip_secrets(&self, entity: &Entity, row: &mut Value) {
        if let Some(p) = self.policies.get(&entity.name) {
            for f in &entity.fields {
                if p.fields.get(&f.name).is_some_and(|v| v["secret"] == true) {
                    row.as_object_mut().unwrap().remove(&f.json);
                }
            }
        }
    }
    pub(crate) fn resolve_account(
        &mut self,
        account: &[u8],
        bound: Option<&[u8]>,
        selector: &str,
    ) -> Result<Value> {
        let rows = self.memberships(account, bound, selector, true)?;
        let selected = bound.is_some() || !selector.trim().is_empty();
        match rows.as_slice() {
            [h] => Ok(
                json!({"actor":value::bytes(&h["id"])?,"tenant":value::bytes(&h["tenant"]["id"])?,"name":format!("@{}/{}",h["tenant"]["alias"].as_str().unwrap_or(""),h["alias"].as_str().unwrap_or(""))}),
            ),
            [] if selected => Err(Fault::denied(
                "account is not a member of the selected tenant",
            )),
            [] => Err(Fault::precondition("account has no membership")),
            _ if selected => Err(Fault::precondition(
                "account has multiple memberships in this tenant",
            )),
            _ => Err(Fault::invalid("select one of the account's tenants")),
        }
    }
}

/// Read predicates attached by an app-owned account-only method to a derived request context.
#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
pub struct AccountReads(std::collections::BTreeMap<String, Vec<ReadFilter>>);
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum ReadFilter {
    /// Compare a persisted UUID field with the authenticated account.
    AccountField(String),
    /// Restrict keys to identifiers already resolved by application policy.
    Ids(Vec<Vec<u8>>),
}
impl AccountReads {
    pub fn allow(mut self, entity: impl Into<String>, filter: ReadFilter) -> Self {
        self.0.entry(entity.into()).or_default().push(filter);
        self
    }
    pub(crate) fn clauses(
        &self,
        e: &paydirt_model::Entity,
        id: &[u8],
        q: &mut crate::driver::Query,
        dialect: paydirt_model::Dialect,
    ) -> crate::Result<Vec<String>> {
        use crate::driver::Bind;
        use paydirt_model::quote;
        let Some(filters) = self.0.get(&e.name).filter(|f| !f.is_empty()) else {
            return Ok(vec!["1=0".into()]);
        };
        let mut result = Vec::new();
        for f in filters {
            match f {
                ReadFilter::AccountField(field) => {
                    let field = e.field(field).map_err(crate::Fault::invalid)?;
                    if !field.is_column() || !matches!(field.kind.as_str(), "BYTES" | "UUID") {
                        return Err(crate::Fault::invalid(
                            "account scope requires a persisted identifier field",
                        ));
                    }
                    let param = q.param(Bind::Bytes(id.to_vec()), dialect);
                    result.push(format!("r.{}={param}", quote(&field.column())));
                }
                ReadFilter::Ids(ids) if ids.is_empty() => result.push("1=0".into()),
                ReadFilter::Ids(ids) => {
                    if ids.iter().any(|id| id.len() != 16) {
                        return Err(crate::Fault::invalid("account scope requires UUID keys"));
                    }
                    let params = ids
                        .iter()
                        .map(|id| q.param(Bind::Bytes(id.clone()), dialect))
                        .collect::<Vec<_>>()
                        .join(",");
                    result.push(format!(
                        "r.{} IN ({params})",
                        quote(&e.key().map_err(crate::Fault::invalid)?.column())
                    ));
                }
            }
        }
        Ok(result)
    }
}
