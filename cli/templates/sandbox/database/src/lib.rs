use base64::{engine::general_purpose::STANDARD,Engine};
use paydirt_store::{Call,Policies,Scope,Store};
use serde_json::{json,Value};
use sqlite_wasm_vfs::sahpool::{install,OpfsSAHPoolCfg};
use wasm_bindgen::prelude::*;
const MODEL:&str=include_str!("../../../generated/storage/model.json");
const POLICIES:&str=include_str!("../../../generated/storage/policies.json");

#[wasm_bindgen]
pub struct AppDatabase(Store);
#[wasm_bindgen]
impl AppDatabase {
    pub async fn open(tenants:&str)->Result<AppDatabase,JsError>{
        install::<sqlite_wasm_rs::WasmOsCallback>(&OpfsSAHPoolCfg::default(),true).await.map_err(|e|JsError::new(&e.to_string()))?;
        let mut db=Store::open("paydirt.sqlite3",MODEL,POLICIES)?;
        let policies:Policies=serde_json::from_str(POLICIES)?;
        let tenant=policies.iter().find(|(_,p)|p.entity["own"]=="OWN_TENANT").ok_or_else(||JsError::new("missing tenant schema"))?.0;
        for hex in serde_json::from_str::<Vec<String>>(tenants)? {
            let id=STANDARD.encode(hex::decode(&hex)?);
            let call=|method:&str,request:Value|Call{entity:tenant.clone(),method:method.into(),request};
            match db.call(&Scope::Internal,call("Get",json!({"ref":{"id":id}}))) {
                Ok(_)=>{}, Err(f) if f.code=="not_found"=>{db.call(&Scope::Internal,call("Add",json!({"id":id,"alias":hex})))?;},Err(e)=>return Err(e.into())
            }
        }
        Ok(Self(db))
    }
    pub fn execute(&mut self,request:&str)->Result<String,JsError>{
        let (scope,call):(Scope,Call)=serde_json::from_str(request)?;
        if matches!(scope,Scope::Internal){return Err(JsError::new("operator scope is local to the database worker"))}
        Ok(serde_json::to_string(&self.0.call(&scope,call))?)
    }
}
