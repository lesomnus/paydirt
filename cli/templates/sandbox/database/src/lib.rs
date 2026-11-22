use base64::{Engine, engine::general_purpose::STANDARD};
use paydirt_store::{Call, Packet, Policies, Scope, Store};
use serde_json::{Value, json};
use sqlite_wasm_vfs::sahpool::{OpfsSAHPoolCfg, install};
use wasm_bindgen::prelude::*;
const MODEL: &str = include_str!("../../../generated/storage/model.json");
const POLICIES: &str = include_str!("../../../generated/storage/policies.json");

#[wasm_bindgen]
pub struct AppDatabase(Store);
#[wasm_bindgen]
impl AppDatabase {
    pub async fn open(tenants: &str, seeds: Option<String>) -> Result<AppDatabase, JsError> {
        install::<sqlite_wasm_rs::WasmOsCallback>(&OpfsSAHPoolCfg::default(), true)
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;
        let mut db = Store::open("paydirt.sqlite3", MODEL, POLICIES)?.with_resources(
            serde_json::from_str(include_str!("../../../resources.json"))?,
            include_bytes!("../../../generated/storage/descriptors.binpb"),
            include_str!("../../../generated/storage/admission.json"))?.with_trail(serde_json::from_str(include_str!("../../../trail.json"))?, include_bytes!("../../../generated/storage/descriptors.binpb"),include_str!("../../../generated/storage/admission.json"))?;
        let policies: Policies = serde_json::from_str(POLICIES)?;
        let tenant = policies
            .iter()
            .find(|(_, p)| p.entity["own"] == "OWN_TENANT")
            .ok_or_else(|| JsError::new("missing tenant schema"))?
            .0;
        #[derive(serde::Deserialize)]
        struct Seed {
            entity: String,
            row: Value,
        }
        let seeds: Vec<Seed> = serde_json::from_str(seeds.as_deref().unwrap_or("[]"))?;
        let (tenant_rows, rows): (Vec<_>, Vec<_>) = seeds
            .into_iter()
            .map(|s| (s.entity, s.row))
            .partition(|(name, _)| name == tenant);
        db.seed(tenant_rows)?;
        for hex in serde_json::from_str::<Vec<String>>(tenants)? {
            let id = STANDARD.encode(hex::decode(&hex)?);
            let call = |method: &str, request: Value| Call {
                entity: tenant.clone(),
                method: method.into(),
                request,
            };
            match db.call(&Scope::Internal, call("Get", json!({"ref":{"id":id}}))) {
                Ok(_) => {}
                Err(f) if f.code == "not_found" => {
                    let alias = if hex.starts_with(|c: char| c.is_ascii_digit()) {
                        format!("tenant-{hex}")
                    } else {
                        hex
                    };
                    db.call(&Scope::Internal, call("Add", json!({"id":id,"alias":alias})))?;
                }
                Err(e) => return Err(e.into()),
            }
        }
        db.seed(rows)?;
        Ok(Self(db))
    }
    pub fn execute(&mut self, request: &str) -> Result<String, JsError> {
        let packet: Packet = serde_json::from_str(request)?;
        Ok(serde_json::to_string(&self.0.handle(packet))?)
    }
}
