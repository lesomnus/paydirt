use crate::{Fault, Result, Scope, Transaction, value};
use paydirt_model::Entity;
use serde_json::{Value, json};

impl Transaction<'_> {
    /// An outer request records every individual successful write on its SQL connection.
    pub(crate) fn record(
        &mut self,
        e: &Entity,
        method: &str,
        before: Option<&Value>,
        after: Option<&Value>,
    ) -> Result<()> {
        let Some(frame) = self.frame else {
            return Ok(());
        };
        let p = &self.policies[&e.name].entity;
        let row = after
            .or(before)
            .ok_or_else(|| Fault::internal("missing changed row"))?;
        let key = e.key().map_err(Fault::internal)?;
        let object = &row[&key.json];
        let actor = value::encoded(frame.actor.as_deref().unwrap_or(&[0; 16]));
        let tenant = value::encoded(&frame.tenant);
        let object_tenant = self.object_tenant(e, row)?;
        let by = format!("/{}Service/{}", e.name, method);
        let changed: Vec<_> = e
            .fields
            .iter()
            .filter(|f| before.and_then(|r| r.get(&f.json)) != after.and_then(|r| r.get(&f.json)))
            .map(|f| f.number)
            .collect();
        let mut audit = json!({"actorId":actor,"tenantId":object_tenant,"actorTenantId":tenant,"objectId":object,"traceId":value::encoded(&frame.trace),"fields":changed,"domain":p["domain"],"action":frame.action});
        if let Some(id) = &frame.caller {
            audit["callerId"] = value::encoded(id)
        }
        if let Some(id) = &frame.caller_tenant {
            audit["callerTenantId"] = value::encoded(id)
        }
        let outbox = json!({"actorId":actor,"tenantId":tenant,"method":frame.action,"by":by,"objectId":object});
        let mut recorder = Transaction {
            db: self.db,
            model: self.model,
            policies: self.policies,
            scope: &Scope::Internal,
            frame: None,
            failed: None,
        };
        for (own, data) in [("OWN_AUDIT", audit), ("OWN_OUTBOX", outbox)] {
            if own == "OWN_AUDIT" && p["trail"].get("off").is_some() {
                continue;
            }
            let name = self
                .policies
                .iter()
                .find(|(_, p)| p.entity["own"] == own)
                .ok_or_else(|| Fault::internal(format!("missing {own} schema")))?
                .0;
            let entity = self.model.entity(name).map_err(Fault::internal)?;
            recorder.insert(entity, &data)?;
        }
        Ok(())
    }
}
