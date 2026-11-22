//! Mirror the billing source's versioned grants. A limit locks its grant rows
//! until the enclosing application write commits.
use crate::{Fault, Frame, Result, Scope, Store, Transaction, meter::valid_key, value};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Allowance {
    None,
    Off,
    On,
    Limited(i64),
    Unlimited,
}
impl Allowance {
    pub fn has(&self) -> bool {
        matches!(self, Self::On | Self::Unlimited) || matches!(self,Self::Limited(n) if *n>0)
    }
    pub fn allows(&self, count: i64) -> bool {
        match self {
            Self::Unlimited => true,
            Self::Limited(n) => count <= *n,
            _ => count <= 0,
        }
    }
}
#[derive(Default, Debug, Serialize)]
pub struct Synced {
    pub applied: usize,
    pub heard: usize,
    pub refused: usize,
    pub forgotten: usize,
}
fn stamp(v: &Value) -> Result<DateTime<Utc>> {
    v.as_str()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Fault::invalid("invalid grant time"))
}
fn unsigned(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str()?.parse().ok())
}
fn grant(raw: &Value) -> Result<Value> {
    let id = value::bytes(&raw["id"])?;
    let tenant = value::bytes(&raw["tenantId"])?;
    if !(1..=64).contains(&id.len())
        || !paydirt::has_domain(&tenant, 1)
        || !raw["key"].as_str().is_some_and(valid_key)
    {
        return Err(Fault::invalid("invalid grant identity"));
    }
    let ver = unsigned(&raw["version"])
        .filter(|&n| n > 0)
        .ok_or_else(|| Fault::invalid("positive grant version required"))?;
    let (kind, amount) = match (raw.get("on"), raw.get("limit"), raw.get("unlimited")) {
        (Some(v), None, None) => (
            if v.as_bool()
                .ok_or_else(|| Fault::invalid("invalid switch"))?
            {
                2
            } else {
                1
            },
            0,
        ),
        (None, Some(v), None) => (
            3,
            unsigned(v)
                .and_then(|n| i64::try_from(n).ok())
                .ok_or_else(|| Fault::invalid("invalid limit"))?,
        ),
        (None, None, Some(v)) if v.is_object() => (4, 0),
        _ => return Err(Fault::invalid("one grant value required")),
    };
    stamp(&raw["dateExpires"])?;
    if let Some(v) = raw.get("dateEffective") {
        stamp(v)?;
    }
    Ok(
        json!({"tenant":{"id":raw["tenantId"]},"key":raw["key"],"value":kind,"amount":amount.to_string(),
        "grantId":raw["id"],"grantVersion":ver.to_string(),"dateEffective":raw["dateEffective"],"dateExpires":raw["dateExpires"]}),
    )
}
impl Store {
    /// Consume the complete list before applying any grant or forgetting an ended one.
    pub fn reconcile_grants(
        &mut self,
        only: Option<&[u8]>,
        mut list: impl FnMut(&str) -> Result<Value>,
    ) -> Result<Synced> {
        if only.is_some_and(|id| !paydirt::has_domain(id, 1)) {
            return Err(Fault::invalid("invalid tenant"));
        }
        let mut seen = BTreeSet::new();
        let mut grants = Vec::new();
        let mut cursor = String::new();
        loop {
            let page = list(&cursor)?;
            let items = page["items"]
                .as_array()
                .ok_or_else(|| Fault::invalid("incomplete grant list"))?;
            grants.extend_from_slice(items);
            let next = page["next"].as_str().unwrap_or("");
            if next.is_empty() {
                break;
            }
            if !seen.insert(next.to_owned()) {
                return Err(Fault::invalid("grant list cursor repeated"));
            }
            cursor = next.to_owned();
        }
        self.apply_grants(only, &grants, true)
    }
    /// Accept incremental source events only after its ready signal and initial reconciliation.
    pub fn follow_grants(
        &mut self,
        events: impl IntoIterator<Item = Result<Value>>,
        mut list: impl FnMut(&str) -> Result<Value>,
    ) -> Result<()> {
        let mut stream = events.into_iter();
        if !stream
            .next()
            .transpose()?
            .is_some_and(|v| v.get("ready").is_some())
        {
            return Err(Fault::precondition("grant stream did not begin with ready"));
        }
        self.reconcile_grants(None, &mut list)?;
        for v in stream {
            let event = v?;
            if let Some(g) = event.get("grant") {
                self.apply_grants(None, std::slice::from_ref(g), false)?;
            } else {
                return Err(Fault::invalid("unknown grant stream event"));
            }
        }
        Ok(())
    }
    pub fn apply_grants(
        &mut self,
        only: Option<&[u8]>,
        source: &[Value],
        complete: bool,
    ) -> Result<Synced> {
        self.transaction(&Scope::Internal,|tx|{
            let entity=tx.owned_entity("OWN_ENTITLEMENT")?.clone();
            let tenants=tx.owned_entity("OWN_TENANT")?.clone();
            let mut result=Synced::default();let mut listed=BTreeSet::new();
            // Serialize source updates and reconciliation, including insertion of a previously unseen grant.
            if tx.db.dialect()==paydirt_model::Dialect::Postgres{tx.db.rows(crate::driver::Query::new("SELECT CAST(pg_advisory_xact_lock(hashtextextended('paydirt.entitlements',0)) AS TEXT) AS value"))?;}
            for raw in source{
                if let Some(id)=raw["id"].as_str(){listed.insert(id.to_owned());}
                let Ok(mut update)=grant(raw) else{result.refused+=1;continue};
                let tenant=value::bytes(&update["tenant"]["id"])?;
                if only.is_some_and(|t|t!=tenant){result.refused+=1;continue}
                match tx.lookup(&tenants,&update["tenant"],true){Ok(_)=>{},Err(f) if f.code=="not_found"=>continue,Err(f)=>return Err(f)}
                let old=tx.matching(&entity,&[("grant_id",&update["grantId"])],true)?.pop();
                if let Some(row)=&old{
                    if row["key"]!=update["key"] || row["tenant"]!=update["tenant"]{result.refused+=1;continue}
                    if unsigned(&row["grantVersion"])>=unsigned(&update["grantVersion"]){result.heard+=1;continue}
                    update["ref"]=json!({"id":row["id"]});update["dateUpdated"]=row["dateUpdated"].clone();
                    if update["dateEffective"].is_null(){update.as_object_mut().unwrap().remove("dateEffective");update["dateEffectiveNull"]=json!(true);}
                }
                let frame=deployment(tenant,"payday.entitle.apply");let mut w=Transaction{admission:None,origin:None,seen:None,changes:tx.changes,outbox:tx.outbox,trail:tx.trail,resources:tx.resources, topic_policy:tx.topic_policy, inputs:tx.inputs, namer:tx.namer,db:tx.db,model:tx.model,policies:tx.policies,scope:&Scope::Internal,frame:Some(&frame),failed:None};
                if old.is_some(){w.patch(&entity,&update,"Patch")?;}else{w.insert(&entity,&update)?;}result.applied+=1;
            }
            if complete{
                let owner=only.map(|t|json!({"id":value::encoded(t)}));
                let fields:Vec<_>=owner.as_ref().map(|v|("tenant",v)).into_iter().collect();
                let now=Utc::now();
                for row in tx.matching(&entity,&fields,true)?{
                    if !listed.contains(row["grantId"].as_str().unwrap()) && stamp(&row["dateExpires"])?<=now{
                        let frame=deployment(value::bytes(&row["tenant"]["id"])?,"payday.entitle.forget");
                        let mut w=Transaction{admission:None,origin:None,seen:None,changes:tx.changes,outbox:tx.outbox,trail:tx.trail,resources:tx.resources, topic_policy:tx.topic_policy, inputs:tx.inputs, namer:tx.namer,db:tx.db,scope:&Scope::Internal,model:tx.model,policies:tx.policies,frame:Some(&frame),failed:None};
                        w.dispatch(&entity,"Erase",&json!({"id":row["id"]}))?;result.forgotten+=1;
                    }
                }
            }Ok(result)
        })
    }
}
pub(crate) fn deployment(tenant: Vec<u8>, action: &str) -> Frame {
    Frame {
        tenant,
        actor: None,
        account: None,
        topics: Default::default(),
        caller: None,
        caller_tenant: None,
        presence: None,
        tenants: None,
        sets: None,
        trace: vec![],
        action: action.into(),
    }
}
impl Transaction<'_> {
    pub fn entitlement(&mut self, key: &str, lock: bool, at: DateTime<Utc>) -> Result<Allowance> {
        if !valid_key(key) {
            return Err(Fault::invalid("invalid entitlement key"));
        }
        let tenant = self
            .frame
            .map(|f| f.tenant.as_slice())
            .or_else(|| match self.scope {
                Scope::Tenant(t) => Some(t.as_slice()),
                _ => None,
            })
            .filter(|id| paydirt::has_domain(id, 1))
            .ok_or_else(|| Fault::precondition("entitlement requires the calling tenant"))?;
        let entity = self.owned_entity("OWN_ENTITLEMENT")?.clone();
        // A missing grant also needs serialization with an arriving grant.
        if lock {
            let owner = self.owned_entity("OWN_TENANT")?.clone();
            self.lookup(&owner, &json!({"id":value::encoded(tenant)}), true)?;
        }
        let rows = self.matching(
            &entity,
            &[
                ("tenant", &json!({"id":value::encoded(tenant)})),
                ("key", &json!(key)),
            ],
            lock,
        )?;
        let mut chosen = None;
        for row in rows {
            let start = row
                .get("dateEffective")
                .filter(|v| !v.is_null())
                .map(stamp)
                .transpose()?
                .unwrap_or(DateTime::<Utc>::MIN_UTC);
            if start > at || stamp(&row["dateExpires"])? <= at {
                continue;
            }
            let rank = (start, value::bytes(&row["grantId"])?);
            if chosen.as_ref().is_none_or(|(old, _)| rank > *old) {
                chosen = Some((rank, row))
            }
        }
        let val = if let Some((_, r)) = chosen {
            match r["value"].as_i64() {
                Some(1) => Allowance::Off,
                Some(2) => Allowance::On,
                Some(3) => Allowance::Limited(
                    r["amount"]
                        .as_str()
                        .and_then(|s| s.parse().ok())
                        .ok_or_else(|| Fault::internal("invalid stored amount"))?,
                ),
                Some(4) => Allowance::Unlimited,
                _ => Allowance::None,
            }
        } else {
            Allowance::None
        };
        if lock && matches!(val, Allowance::On | Allowance::Off) {
            return Err(Fault::precondition("a switch cannot be read as a limit"));
        }
        Ok(val)
    }
}
