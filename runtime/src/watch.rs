use crate::{
    Fault, Result, Transaction,
    driver::{Bind, Driver, Query},
};
use paydirt_model::{Dialect, Entity};
use serde_json::{Value, json};

pub(crate) fn install(db: &mut dyn Driver) -> Result<()> {
    db.batch("CREATE TABLE IF NOT EXISTS paydirt_watch_clock (id INTEGER PRIMARY KEY, revision BIGINT NOT NULL);
        INSERT INTO paydirt_watch_clock (id,revision) VALUES (1,0) ON CONFLICT (id) DO NOTHING;
        CREATE TABLE IF NOT EXISTS paydirt_watch_state (entity TEXT NOT NULL, object_id TEXT NOT NULL, revision BIGINT NOT NULL, action TEXT NOT NULL, PRIMARY KEY (entity,object_id));")
}
impl Transaction<'_> {
    pub(crate) fn changed(&mut self, e: &Entity, row: &Value, method: &str) -> Result<()> {
        if !self
            .policies
            .get(&e.name)
            .is_some_and(|p| p.entity.get("watch").is_some())
        {
            return Ok(());
        }
        let id = row[&e.key().map_err(Fault::internal)?.json]
            .as_str()
            .ok_or_else(|| Fault::internal("missing watch identifier"))?;
        let action = self
            .frame
            .map(|f| f.action.clone())
            .unwrap_or_else(|| format!("/{}Service/{method}", e.name));
        let mut q = Query::new("");
        let d = self.db.dialect();
        let entity = q.param(Bind::Text(e.name.clone()), d);
        let id = q.param(Bind::Text(id.into()), d);
        let action = q.param(Bind::Text(action), d);
        // Updating a shared clock serializes commit order across server processes.
        self.db
            .batch("UPDATE paydirt_watch_clock SET revision=revision+1 WHERE id=1")?;
        q.sql=format!("INSERT INTO paydirt_watch_state (entity,object_id,revision,action) VALUES ({entity},{id},(SELECT revision FROM paydirt_watch_clock WHERE id=1),{action})
            ON CONFLICT (entity,object_id) DO UPDATE SET revision=excluded.revision,action=excluded.action");
        self.db.exec(q)?;
        Ok(())
    }
    fn watch_clock(&mut self) -> Result<i64> {
        let lock = if self.db.dialect() == Dialect::Postgres {
            " FOR SHARE"
        } else {
            ""
        };
        self.db
            .rows(Query::new(format!(
                "SELECT CAST(revision AS TEXT) AS value FROM paydirt_watch_clock WHERE id=1{lock}"
            )))?
            .first()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| Fault::internal("missing watch clock"))
    }
    pub(crate) fn watch_start(&mut self, e: &Entity, req: &Value) -> Result<Value> {
        let fs = req["filters"]
            .as_array()
            .filter(|fs| !fs.is_empty() && fs.len() <= 32)
            .ok_or_else(|| Fault::invalid("watch requires between one and 32 filters"))?;
        let rev = self.watch_clock()?;
        let key = e.key().map_err(Fault::internal)?;
        let mut ids = Vec::new();
        for f in fs {
            let row = self.lookup(
                e,
                f.get("ref")
                    .ok_or_else(|| Fault::invalid("watch requires named references"))?,
                false,
            )?;
            let id = &row[&key.json];
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        let mut pages = Vec::new();
        if req["skipSnapshot"] != true {
            let mut after = json!("");
            loop {
                let list = self.list(e, &json!({"filters":fs,"after":after}))?;
                let items = list["items"]
                    .as_array()
                    .ok_or_else(|| Fault::internal("invalid snapshot"))?
                    .iter()
                    .map(|v| json!({"id":v[&key.json],"value":v}))
                    .collect::<Vec<_>>();
                if !items.is_empty() {
                    pages.push(json!({"items":items}));
                }
                after = list["next"].clone();
                if after == "" {
                    break;
                }
            }
        }
        Ok(json!({"revision":rev,"ids":ids,"pages":pages}))
    }
    pub(crate) fn watch_poll(&mut self, e: &Entity, req: &Value) -> Result<Value> {
        let rev = self.watch_clock()?;
        let mut q = Query::new("");
        let d = self.db.dialect();
        let ids = req["ids"]
            .as_array()
            .filter(|v| !v.is_empty() && v.len() <= 32)
            .ok_or_else(|| Fault::invalid("invalid watch keys"))?;
        let params = ids
            .iter()
            .map(|id| {
                id.as_str()
                    .map(|s| q.param(Bind::Text(s.into()), d))
                    .ok_or_else(|| Fault::invalid("invalid watch key"))
            })
            .collect::<Result<Vec<_>>>()?
            .join(",");
        let entity = q.param(Bind::Text(e.name.clone()), d);
        let since = q.param(
            Bind::Big(
                req["revision"]
                    .as_i64()
                    .ok_or_else(|| Fault::invalid("invalid revision"))?,
            ),
            d,
        );
        let array = if d == Dialect::Postgres {
            "CAST(json_build_array(object_id,action) AS TEXT)"
        } else {
            "json_array(object_id,action)"
        };
        q.sql = format!(
            "SELECT {array} AS value FROM paydirt_watch_state WHERE entity={entity} AND object_id IN ({params}) AND revision>{since} ORDER BY revision"
        );
        let mut items = Vec::new();
        for raw in self.db.rows(q)? {
            let [id, action]: [String; 2] =
                serde_json::from_str(&raw).map_err(|_| Fault::internal("invalid watch event"))?;
            let mut event = json!({"id":id,"action":action});
            match self.lookup(e, &json!({&e.key:id}), false) {
                Ok(row) => event["value"] = self.image(e, row, &Value::Null)?,
                Err(err) if err.code == "not_found" => {}
                Err(err) => return Err(err),
            }
            items.push(event);
        }
        Ok(json!({"revision":rev,"items":items}))
    }
}
