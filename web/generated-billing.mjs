import assert from "node:assert/strict";
import {execFileSync} from "node:child_process";

// The billing source uses the same protobuf JSON fields as Payday's EntitlementGrant.
export async function billingCases(app,cwd,env,robot,primary,other){
 const run=(entity,method,data)=>JSON.parse(execFileSync(app,[entity,method,JSON.stringify(data)],{cwd,env,encoding:"utf8",stdio:["ignore","pipe","pipe"]}));
 const own={headers:{authorization:"Bearer "+primary.token}},foreign={headers:{authorization:"Bearer "+other.token}};
 const tid=primary.id.toString("base64"),otherId=other.id.toString("base64");
 const measurements=run("app.Tenant","Measure",{id:tid});
 assert.equal(measurements.errors.length,0);
 const readings=Object.fromEntries(measurements.readings.map(r=>[r.meter,r]));
 assert.equal(readings["apptest.seats"].value,2);assert.equal(readings["apptest.seats"].unit,"{seat}");
 assert.equal(readings["apptest.robots.attested"].value,0);
 assert.equal(new Set(measurements.readings.map(r=>r.measuredAt)).size,1);
 assert.equal(run("app.Tenant","Measure",{id:otherId}).readings.find(r=>r.meter==="apptest.seats").value,1);
 const key="fleet.capacity",early={id:"YQ==",tenantId:tid,key,version:"2",limit:"5",dateExpires:"2099-01-01T00:00:00Z"};
 const later={...early,id:"Yg==",limit:"1",version:"1",dateEffective:"2020-01-01T00:00:00Z"};
 const sync=(...pages)=>run("app.Entitlement","Sync",{tenantId:tid,pages});
 const quota=(opts=own)=>robot.allowance({key,lock:true},opts);
 assert.equal(sync({items:[early]}).applied,1);assert.equal((await quota()).allowsTwo,true);assert.equal((await quota(foreign)).enabled,false);
 assert.equal(sync({items:[{...early,version:"1",limit:"0"}]}).heard,1);assert.equal((await quota()).allowsTwo,true);
 assert.throws(()=>sync({items:[later],next:"unfinished"}));assert.equal((await quota()).allowsTwo,true);
 assert.equal(sync({items:[early],next:"p2"},{items:[later]}).applied,1);assert.equal((await quota()).allowsTwo,false);
 assert.equal(sync({items:[]}).forgotten,0);
 assert.equal(sync({items:[{...later,dateExpires:"2021-01-01T00:00:00Z",version:"2"}]}).applied,1);
 assert.equal((await quota()).allowsTwo,true);assert.equal(sync({items:[]}).forgotten,1);
 const bad={...early,version:"3",tenantId:otherId};
 assert.equal(sync({items:[bad]}).refused,1);
 assert.throws(()=>run("app.Entitlement","Follow",{events:[{grant:early}],pages:[]}));
 run("app.Entitlement","Follow",{events:[{ready:{}},{grant:{...early,version:"3",limit:"0"}}],pages:[{items:[early]}]});
 assert.equal((await quota()).enabled,false);
}

