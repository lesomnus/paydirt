export async function notificationCases(service,account,tenant,actor,opts){
  const group={key:{case:"name",value:"jobs"}}, hidden={key:{case:"name",value:"private"}};
  const svc=service, reader=opts("notices"), publisher=opts("publisher");
  const req={tenantId:tenant,topics:[group]};
  const add=async(key,topic=group,source=actor)=>svc.add({tenantId:tenant,topic,kind:"job.done",sourceId:source,idempotencyKey:key},publisher);
  const eq=(a,b,why)=>{if(JSON.stringify(a)!==JSON.stringify(b))throw Error(why+" "+JSON.stringify(a)+" / "+JSON.stringify(b))};
  const rejected=async(call,code)=>{try{await call}catch(e){eq(e.code,code,"unexpected error "+e);return}throw Error("expected RPC failure "+code)};
  const ids=vs=>vs.map(v=>String(v.id));
  const oldest=await add("first");const again=await add("first");eq(String(again.id),String(oldest.id),"publication retry");
  await rejected(add("first",hidden),6);
  const next=await add("second"),other=await add("hidden",hidden);
  eq(String((await svc.get({ref:{id:oldest.id}},opts("manyNotices"))).id),String(oldest.id),"Get must use the full tenant grant");
  await rejected(svc.get({ref:{id:oldest.id}},opts("foreignNotices")),5);
  await rejected(svc.get({ref:{id:other.id}},opts("manyNotices")),5);
  await rejected(svc.get({ref:{id:other.id}},reader),5);
  await rejected(svc.read({...req,topics:[group,hidden]},reader),7);
  const mark=(rows,opt=reader)=>svc.mark({tenantId:tenant,refs:rows.map(v=>({id:v.id}))},opt);
  await rejected(mark([oldest,other]),5);
  eq((await svc.count({...req,filter:{unreadOnly:true}},reader)).count.toString(),"2","failed mark changed receipts");
  const firstPage=await svc.read({...req,size:1},reader);
  if(!firstPage.next)throw Error("missing notification cursor");
  await rejected(svc.read({...req,cursor:firstPage.next,filter:{kinds:["job.done"]}},reader),3);
  const secondPage=await svc.read({...req,cursor:firstPage.next},reader);
  eq(ids(secondPage.items.map(x=>x.notification)),ids([oldest]),"next page");
  const marked=await mark([next]);const retry=await mark([next]);
  eq(String(marked.items[0].id),String(retry.items[0].id),"receipt changed");eq(String(retry.items[0].dateRead.seconds),String(marked.items[0].dateRead.seconds),"first read lost");
  const unread=await svc.read({...req,size:1,filter:{kinds:["job.done"],sourceIds:[actor],unreadOnly:true}},reader);
  eq(ids(unread.items.map(i=>i.notification)),ids([oldest]),"filter before page");eq(unread.next,"","trailing cursor");
  const counts=await svc.count(req,reader);eq([String(counts.count),String(counts.sourceCount)],["2","1"],"distinct sources");
  await rejected(mark([]),3);await rejected(mark([oldest,oldest]),3);
  const control=new AbortController(),stream=svc.listen(req,{...reader,signal:control.signal,timeoutMs:12000})[Symbol.asyncIterator]();
  try{
    eq((await stream.next()).value.event.case,"ready","listen was not ready");
    await add("after-ready");eq((await stream.next()).value.event.case,"invalidated","missing invalidation");
  }finally{control.abort();await stream.return?.()}
  const as=opts("noMember");
  eq((await account.read({topics:[group]},as)).items.length,0,"account read tenant notices");
  await rejected(account.read({topics:[group],tenantId:tenant},as),3);
}

