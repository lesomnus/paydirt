// The same generated TypeScript service exercises Go gRPC, Rust Connect and Rust dRPC.
export async function queryScenario(robot, tenantId, options) {
  const ref = id => ({key:{case:"id",value:id}});
  const tenant=ref(tenantId);
  const rows=[];
  for(const alias of ["query-c","query-a","query-b"]){
    rows.push(await robot.add({tenant,alias},options));
  }
  const filters=rows.map(row=>({ref:ref(row.id)}));
  async function code(fn){
    try{await fn();return 0;}catch(e){return e.code;}
  }
  const report={};
  for(const [name,order,reverse] of [["default",0,false],["alias",1,false],["descending",2,false],["reverse",1,true]]){
    const aliases=[];
    let after="";
    do{
      const page=await robot.list({filters,size:1,order,reverse,after},options);
      if(page.items.length!==1 || new Set(aliases).has(page.items[0].alias))throw Error("invalid cursor boundary");
      aliases.push(page.items[0].alias);
      if(aliases.length>3)throw Error("unbounded cursor");
      after=page.next;
      if(name==="reverse" && after)report.wrongOrder=await code(()=>robot.list({filters,size:1,order,after},options));
    }while(after);
    report[name]=aliases;
  }
  report.badFilter=await code(()=>robot.list({filters:[{}]},options));
  report.badCursor=await code(()=>robot.list({filters,after:"invalid!"},options));
  report.noAuth=await code(()=>robot.get({ref:ref(rows[0].id)}));
  const absent=new Uint8Array(rows[0].id);absent[15]^=127;
  report.missing=await code(()=>robot.get({ref:ref(absent)},options));
  const controller=new AbortController();
  const opts={...options,signal:controller.signal,timeoutMs:15000};
  const slug={key:{case:"slug",value:{tenant,alias:"query-b"}}};
  const stream=robot.watch({filters:[{ref:slug}]},opts)[Symbol.asyncIterator]();
  try {
    const snap=(await stream.next()).value.items;
    report.snapshot=snap.map(item=>[item.value.alias,item.action]);
    await robot.patch({ref:ref(rows[2].id),alias:"query-renamed",dateUpdated:rows[2].dateUpdated},options);
    const changed=(await stream.next()).value.items;
    report.change=changed.map(item=>[item.value.alias,item.action]);
    await robot.erase(ref(rows[2].id),options);
    const removed=(await stream.next()).value.items;
    report.removal=removed.map(item=>[!!item.value,item.action]);
    controller.abort();
    report.canceled=await code(()=>stream.next());
  } finally {controller.abort();await stream.return?.();}
  const choice=(name,value)=>({case:name,value});
  const scope=choice("targets",{selectors:[{kind:choice("key",{kind:choice("field",{number:4})})}]});
  const str=value=>({kind:choice("s",value)});
  const entries=[
    {scope,kind:choice("test",{want:choice("value",str("query-a"))})},
    {scope,kind:choice("assign",{value:str("query-applied")})},
  ];
  const request={ref:ref(rows[1].id),patch:{messageType:"app.Robot",delta:{entries}}};
  report.applied=(await robot.apply(request,options)).alias;
  report.precondition=await code(()=>robot.apply(request,options));
  return report;
}
