use crate::{Fault, Result, Scope, Transaction, value};
use paydirt_model::Entity;
use serde_json::{Value, json};

impl Transaction<'_> {
    /// An outer request records every individual successful write on its SQL connection.
    pub(crate) fn record(
        &mut self,
        e: &Entity,
        method: &str,
        resource_method: &str,
        request: &Value,
        before: Option<&Value>,
        after: Option<&Value>,
    ) -> Result<()> {
        let row = after
            .or(before)
            .ok_or_else(|| Fault::internal("missing changed row"))?;
        if let Some(rules) = self.admission {
            rules.check(
                self.policies,
                &e.name,
                method,
                &row[&e.key().map_err(Fault::internal)?.json],
                self.frame,
                self.origin.as_ref(),
                before,
                after.filter(|_| method != "Erase"),
            )?;
        }
        self.changed(e, row, method)?;
        let policy = self
            .policies
            .get(&e.name)
            .map(|p| &p.entity)
            .unwrap_or(&Value::Null);
        let by = format!("/{}Service/{resource_method}", e.name);
        let mut resource = String::new();
        if let Some(recorder) = self.resources.filter(|r| r.selected(&e.name)) {
            let outer = self.frame.map_or(by.as_str(), |f| &f.action);
            let domain =
                self.policies
                    .values()
                    .find(|p| p.entity["own"] == "OWN_OUTBOX")
                    .and_then(|p| p.entity["domain"].as_u64())
                    .ok_or_else(|| Fault::internal("missing outbox domain"))? as u8;
            resource = recorder.record(
                &e.name,
                &by,
                outer,
                before,
                after.filter(|_| method != "Erase"),
                value::encoded(&(self.inputs.mint)(domain)),
                self.inputs.stamp(),
            )?;
        }
        if !matches!(
            policy["own"].as_str(),
            Some("OWN_AUDIT" | "OWN_ACCESS" | "OWN_OUTBOX")
        ) && let Ok(id) = value::bytes(&row[&e.key().map_err(Fault::internal)?.json])
            && let Some(key) = paydirt::format_id(&id)
        {
            self.changes.push(paydirt::watch::Change {
                method: self.frame.map_or_else(|| by.clone(), |f| f.action.clone()),
                resource: resource.clone(),
                by: by.clone(),
                key,
                routes: Some(Vec::new()),
                ..Default::default()
            });
        }
        let Some(frame) = self.frame else {
            return self.observe_write(e, method, before, after);
        };
        let p = &self.policies[&e.name].entity;
        let key = e.key().map_err(Fault::internal)?;
        let object = &row[&key.json];
        let actor = value::encoded(frame.actor.as_deref().unwrap_or(&[0; 16]));
        let tenant = value::encoded(if frame.tenant.is_empty() {
            &[0; 16]
        } else {
            &frame.tenant
        });
        let object_tenant = self.object_tenant(e, row)?;
        let filed = if method == "Erase" && after.is_none() {
            &tenant
        } else {
            &object_tenant
        };
        let audit_on = p["trail"].get("off").is_none()
            && self
                .trail
                .and_then(|r| r.filter.as_deref())
                .is_none_or(|filter| filter(&e.name, method));
        let by = format!("/{}Service/{}", e.name, method);
        let changed: Vec<_> = e
            .fields
            .iter()
            .filter(|f| before.and_then(|r| r.get(&f.json)) != after.and_then(|r| r.get(&f.json)))
            .map(|f| f.number)
            .collect();
        let mut audit = json!({"actorId":actor,"tenantId":filed,"actorTenantId":tenant,"objectId":object,"traceId":value::encoded(&frame.trace),"fields":changed,"domain":p["domain"],"action":frame.action});
        if let Some(recorder) = self.trail {
            recorder.content(e, request, before, after, &mut audit)?;
        }
        if let Some(other) = &frame.counterpart {
            if !paydirt::has_domain(other, 1) {
                return Err(Fault::invalid("counterpart must identify a tenant"));
            }
            let other = value::encoded(other);
            if &other != filed {
                audit["counterpartTenantId"] = other;
            }
        }
        if let Some(id) = &frame.caller {
            audit["callerId"] = value::encoded(id)
        }
        if let Some(reference) = &frame.presence {
            audit["presence"] = json!(reference);
        }
        if let Some(id) = &frame.caller_tenant {
            audit["callerTenantId"] = value::encoded(id)
        }
        let mut outbox = json!({"actorId":actor,"tenantId":tenant,"method":frame.action,"by":by,"objectId":object});
        if !resource.is_empty() {
            outbox["event"] = json!(resource)
        }
        if audit_on {
            self.sign_write(e, after, &mut audit)?;
        }
        if let Some(rec) = self.trail.filter(|r| r.options.access)
            && let Some(groups) = self.pending_access.as_deref_mut()
        {
            let mut written = crate::access::Subjects::new();
            let owner = object_tenant.as_str().unwrap_or_default();
            if let Some(image) = after.or(before) {
                rec.subjects(
                    self.policies,
                    &e.name,
                    image,
                    false,
                    owner,
                    &mut written,
                    false,
                    0,
                )?;
            }
            // Clearing a personal field writes the subject even when its postimage is empty.
            if written.is_empty()
                && e.fields.iter().any(|f| {
                    self.policies[&e.name]
                        .fields
                        .get(&f.name)
                        .is_some_and(|p| p["personal"] == true)
                        && (request.get(&f.json).is_some()
                            || request.get(format!("{}Null", f.json)).is_some())
                })
            {
                if let Some(subject) = object.as_str() {
                    written
                        .entry(owner.into())
                        .or_default()
                        .insert(subject.into());
                }
            }
            for (owner, ids) in written {
                groups.entry(owner).or_default().extend(ids);
            }
        }
        let mut recorder = Transaction {
            scope_rules: self.scope_rules,
            pending_access: None,
            admission: None,
            origin: None,
            seen: None,
            changes: self.changes,
            outbox: self.outbox,
            trail: self.trail,
            resources: self.resources,
            topic_policy: self.topic_policy,
            inputs: self.inputs,
            namer: self.namer,
            db: self.db,
            model: self.model,
            policies: self.policies,
            scope: &Scope::Internal,
            frame: None,
            failed: None,
        };
        for (own, data) in [("OWN_AUDIT", audit), ("OWN_OUTBOX", outbox)] {
            if (own == "OWN_AUDIT" && !audit_on) || (own == "OWN_OUTBOX" && !recorder.outbox) {
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
        self.observe_write(e, method, before, after)
    }
    fn observe_write(
        &self,
        e: &Entity,
        method: &str,
        before: Option<&Value>,
        after: Option<&Value>,
    ) -> Result<()> {
        if let Some(rec) = self.trail {
            for observer in &rec.observers {
                observer(&crate::audit_content::Write {
                    entity: &e.name,
                    method,
                    previous: before,
                    value: after,
                })?;
            }
        }
        Ok(())
    }
}
