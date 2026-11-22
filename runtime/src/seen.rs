//! Successful, primary-key reference resolutions within one frame's SQL transaction.
use crate::{Result, Transaction};
use paydirt_model::Entity;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct Seen(BTreeMap<(String, String), Value>);
impl Seen {
    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }
}
impl Transaction<'_> {
    pub(crate) fn reference_row(&mut self, e: &Entity, reference: &Value) -> Result<Value> {
        let key_name = &e.key().map_err(crate::Fault::internal)?.json;
        let key = reference
            .as_object()
            .filter(|o| o.len() == 1)
            .and_then(|v| v.get(key_name))
            .and_then(Value::as_str);
        let Some(key) = key.filter(|_| self.seen.is_some()) else {
            return self.read_reference(e, reference);
        };
        let entry = (e.name.clone(), key.to_owned());
        if let Some(row) = self.seen.as_deref().and_then(|cache| cache.0.get(&entry)) {
            return Ok(row.clone());
        }
        let row = self.read_reference(e, reference)?;
        if let Some(cache) = self.seen.as_mut() {
            cache.0.insert(entry, row.clone());
        }
        Ok(row)
    }
    fn read_reference(&mut self, entity: &Entity, reference: &Value) -> Result<Value> {
        #[cfg(not(target_arch = "wasm32"))]
        let timer = paydirt::slow::Timer::start();
        let row = self.lookup(entity, reference, false);
        #[cfg(not(target_arch = "wasm32"))]
        timer.finish(|| format!("/{}Service/Get", entity.name).into());
        row
    }
    // A change to an owning edge can change a descendant's visibility through a
    // multi-hop tenant path even if the descendant itself wasn't updated.
    pub(crate) fn forget_references(&mut self) {
        if let Some(seen) = self.seen.as_mut() {
            seen.clear();
        }
    }
}

#[cfg(all(test, any(feature = "postgres", feature = "sqlite")))]
mod tests {
    use crate::{Call, Connection, Scope, Store, value::encoded};
    use diesel::{Connection as _, connection::InstrumentationEvent};
    use serde_json::json;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    fn exercise(url: &str) {
        let mut db = Store::open(
            url,
            include_str!("../../generated/storage/model.json"),
            include_str!("../../generated/storage/policies.json"),
        )
        .unwrap();
        let tenant = paydirt::mint(1);
        let alias = format!("scope-{}", hex::encode(tenant));
        let cell = paydirt::mint(10);
        let key = json!({"id":encoded(&cell)});
        db.seed(vec![
            (
                "app.Tenant".into(),
                json!({"id":encoded(&tenant),"alias":alias}),
            ),
            (
                "app.Cell".into(),
                json!({"id":encoded(&cell),"tenant":{"id":encoded(&tenant)},"alias":"before"}),
            ),
        ])
        .unwrap();
        let selects = Arc::new(AtomicUsize::new(0));
        let observed = selects.clone();
        let instrument = move |event: InstrumentationEvent<'_>| {
            if let InstrumentationEvent::StartQuery { query, .. } = event
                && query.to_string().contains("FROM \"pd_app__Cell\" AS r")
            {
                observed.fetch_add(1, Ordering::Relaxed);
            }
        };
        match &mut db.db {
            #[cfg(feature = "sqlite")]
            Connection::Sqlite(c) => c.set_instrumentation(instrument),
            #[cfg(feature = "postgres")]
            Connection::Postgres(c) => c.set_instrumentation(instrument),
        }
        let scope = Scope::Tenant(tenant.to_vec());
        let call = |method: &str, request| Call {
            entity: "app.Cell".into(),
            method: method.into(),
            request,
        };
        db.transaction(&scope, |tx| {
            let e = tx.model.entity("app.Cell").unwrap();
            for _ in 0..10 {
                assert_eq!(tx.reference_row(e, &key)?["alias"], "before");
            }
            assert_eq!(selects.load(Ordering::Relaxed), 1);
            tx.call(call("Patch", json!({"ref":key,"alias":"after"})))?;
            let before = selects.load(Ordering::Relaxed);
            assert_eq!(tx.reference_row(e, &key)?["alias"], "after");
            assert_eq!(selects.load(Ordering::Relaxed), before + 1);
            Ok(())
        })
        .unwrap();
        let before = selects.load(Ordering::Relaxed);
        db.transaction(&scope, |tx| {
            let e = tx.model.entity("app.Cell").unwrap();
            tx.reference_row(e, &key)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(selects.load(Ordering::Relaxed), before + 1);
        let other = Scope::Tenant(paydirt::mint(1).to_vec());
        assert_eq!(
            db.transaction(&other, |tx| tx
                .reference_row(tx.model.entity("app.Cell").unwrap(), &key))
                .unwrap_err()
                .code,
            "not_found"
        );
        db.transaction(&scope, |tx| {
            let e = tx.model.entity("app.Cell").unwrap();
            tx.reference_row(e, &key)?;
            tx.call(call("Erase", key.clone()))?;
            assert_eq!(tx.reference_row(e, &key).unwrap_err().code, "not_found");
            Ok(())
        })
        .unwrap();
    }
    #[cfg(feature = "sqlite")]
    #[test]
    fn sqlite() {
        exercise(":memory:")
    }
    #[cfg(feature = "postgres")]
    #[test]
    fn postgres() {
        if let Ok(url) = std::env::var("DATABASE_URL") {
            exercise(&url);
        }
    }
}
