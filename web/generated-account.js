// Mirrors the pinned reference application's account membership, selector and role cases.
const hex = bytes => Array.from(bytes, b => b.toString(16).padStart(2, "0")).join("");
const bytes = s => Uint8Array.from(s.match(/../g), b => parseInt(b, 16));
const b64 = s => btoa(String.fromCharCode(...bytes(s)));
const uuid = s => s.replace(/^(........)(....)(....)(....)(............)$/, "$1-$2-$3-$4-$5");
export function accounts(tenants) {
  const [a,b]=tenants;
  const id=(tenant,tag)=>{const buf=bytes(tenant);buf[9]=2;buf[15]=tag;return hex(buf)};
  const account=id(a,240),one=id(a,241),none=id(a,242);
  const seeds=[[a,"alice",account,1],[b,"alice",account,2],[a,"bob",one,3]].map(([t,alias,acct,tag])=>({
    entity:"app.Holder",row:{id:b64(id(t,tag)),alias,tenant:{id:b64(t)},account:b64(acct)}
  }));
  const principals={member:{account},single:{account:one},noMember:{account:none,topics:{read:["n:jobs"]}},bound:{account,tenant:a},
    notices:{tenant:a,actor:id(a,1),topics:{read:["n:jobs"]}},publisher:{tenant:a,actor:id(a,1),topics:{publish:["*"],producer:"worker"}},
    attenuated:{account,methods:["/app.RobotService/Get"]}};
  const policy={roles:{["@"+a+"/reader"]:{methods:["/app.RobotService/List","/app.RobotService/Watch"]}},
    groups:{["@"+a+"/office"]:{members:["@"+a+"/alice"]}},bindings:[{role:"@"+a+"/reader",group:"@"+a+"/office"}]};
  return {principals,seeds,policy};
}
function check(yes, message){if(!yes)throw Error(message)}
async function code(call,want){try{await call}catch(e){check(e.code===want,"expected "+want+", received "+e);return}throw Error("expected RPC failure "+want)}
export async function accountCases(holder,robot,tenants,restricted=false){
  const [a,b]=tenants;
  const opts=(token,tenant)=>({headers:{authorization:"Bearer "+token,...(tenant?{"paydirt-tenant":tenant}:{})}});
  const list=(token,tenant)=>robot.list({},opts(token,tenant));
  if(restricted) {
    await list("member",uuid(a));
    await code(list("member",uuid(b)),7);
    await code(list("attenuated",uuid(a)),7);
    await code(robot.add({},opts("member",uuid(a))),7);
    return;
  }
  const members=await holder.memberships({},opts("member","@ignored"));
  check(members.items.length===2,"account needs two live memberships");
  check(members.items.map(h=>hex(h.tenant.id)).sort().join() === [a,b].sort().join(),"memberships escaped account predicate");
  check(members.items.every(h=>h.tenant.alias && h.alias==="alice"),"tenant not hydrated");
  check((await holder.memberships({},opts("noMember"))).items.length===0,"unknown account has memberships");
  await code(list("member"),3); await code(list("noMember"),9);
  await code(list("member","@unknown"),7); await code(list("member","@not/a/tenant"),3);
  await code(holder.memberships({},opts("bound")),7);
  await code(list("bound",uuid(b)),7);await code(list("single",uuid(b)),7);
  for(const [token,selector,t] of [["member",uuid(b),b],["member","@"+a.toUpperCase(),a],["bound",undefined,a],["single",undefined,a]]){
    const items=(await list(token,selector)).items;
    check(items.every(row=>hex(row.tenant.id)===t),"foreign tenant returned");
  }
}

