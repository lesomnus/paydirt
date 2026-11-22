import {fromBinary} from "@bufbuild/protobuf";
import {Code} from "@connectrpc/connect";
const ref=id=>({key:{case:"id",value:id}});
export async function resourceCases(registry,resources,cell,robot,opts,foreign,tenant,other,cel){
  const capture=async (filter,act)=>{
    const controller=new AbortController();
    let signalReady;
    const ready=new Promise(resolve=>signalReady=resolve);
    const reader=resources.watch({filter},{...opts,onHeader:signalReady,signal:controller.signal,timeoutMs:8000})[Symbol.asyncIterator]();
    const first=reader.next();first.catch(()=>{});
    try{
      await Promise.race([ready,first.then(()=>{throw Error("resource sent data before a write")})]);
      return await act(first,reader);
    }finally{controller.abort();await reader.return?.().catch(()=>{});}
  };
  const unpack=any=>fromBinary(registry.getMessage(any.typeUrl.split("/").at(-1)),any.value);
  const seen=new Set();
  const check=(event,kind,method)=>{
    const v=event.value;
    if(event.done||v.kind!==kind||!v.dateCreated||v.method!==method||v.callMethod!==method||v.id.length!==16)throw Error("incorrect resource envelope "+method);
    const id=Array.from(v.id).join(",");
    if(seen.has(id))throw Error("repeated event ID");seen.add(id);
    return v;
  };
  const historical=await robot.add({tenant:ref(tenant),alias:"resource-old"},opts);
  const hidden=await robot.add({tenant:ref(other),alias:"resource-foreign"},foreign);
  await capture("",async(first,reader)=>{
    await robot.patch({ref:ref(hidden.id),alias:"resource-foreign-edit",dateUpdated:hidden.dateUpdated},foreign);
    const created=await cell.add({tenant:ref(tenant),alias:"resource-cell"},opts);
    const a=check(await first,1,"/app.CellService/Add");
    if(a.previous||unpack(a.value).alias!=="resource-cell")throw Error("historical or foreign image escaped the wall");
    const row=await robot.add({tenant:ref(tenant),alias:"resource-fresh",secret:new Uint8Array([11,22])},opts);
    const b=check(await reader.next(),1,"/app.RobotService/Add");
    if(b.previous||unpack(b.value).secret.length||unpack(b.value).alias!=="resource-fresh")throw Error("resource public postimage");
    try {await robot.renamePair({first:ref(row.id),second:ref(historical.id),firstAlias:"resource-dupe",secondAlias:"resource-dupe"},opts);throw Error("rollback unexpectedly committed")}catch(e){if(e.code!==Code.AlreadyExists)throw e}
    await robot.patch({ref:ref(row.id),dateUpdated:row.dateUpdated,alias:"resource-after"},opts);
    const c=check(await reader.next(),2,"/app.RobotService/Patch");
    if(unpack(c.previous).alias!=="resource-fresh"||unpack(c.value).alias!=="resource-after")throw Error("resource update images or rollback");
    await cell.erase(ref(created.id),opts);
    const d=check(await reader.next(),3,"/app.CellService/Erase");
    if(d.value||unpack(d.previous).alias!=="resource-cell")throw Error("resource deletion");
  });
  const expression='has(event.value) && type(event.value) == app.Robot && event.kind == payday.CHANGE_KIND_CREATED && event.value.alias.startsWith("picked") && event.value.secret == b""';
  if(cel){
    await capture(expression,async first=>{
      await robot.add({tenant:ref(tenant),alias:"ignored-resource"},opts);
      await robot.add({tenant:ref(tenant),alias:"picked-resource",secret:new Uint8Array([1])},opts);
      if(unpack((await first).value.value).alias!=="picked-resource")throw Error("public CEL filter");
    });
  }
  for(const [filter,code] of cel?[["42",Code.InvalidArgument],["event.",Code.InvalidArgument],["event.method",Code.InvalidArgument]]:[[expression,Code.Unimplemented]]){
    try{await resources.watch({filter},{...opts,timeoutMs:3000})[Symbol.asyncIterator]().next();throw Error("accepted invalid resource filter")}catch(e){if(e.code!==code)throw e}
  }
}
