// Run two independent generated app servers on one tenant's database.
import assert from "node:assert/strict";
import {randomBytes} from "node:crypto";
import {spawn,execFile,execFileSync} from "node:child_process";
import {mkdtemp,readFile,writeFile,rm} from "node:fs/promises";
import {promisify} from "node:util";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {createServer,connect} from "node:net";
import {setTimeout as delay} from "node:timers/promises";
import {createClient} from "@connectrpc/connect";
import {createConnectTransport} from "@connectrpc/connect-web";
import {createFileRegistry,fromBinary} from "@bufbuild/protobuf";
import {FileDescriptorSetSchema} from "@bufbuild/protobuf/wkt";
const app=process.env.GENERATED_APP;
const project=process.env.CELL_PROJECT;
const registry=createFileRegistry(fromBinary(FileDescriptorSetSchema,await readFile(join(project,"generated/storage/descriptors.binpb"))));
const id=randomBytes(16); id[6]=128;id[8]=128;id[9]=1;
const opts={headers:{authorization:"Bearer member"}};
const reference=id=>({key:{case:"id",value:id}});
const ddl=sql=>execFileSync("psql",[process.env.DATABASE_URL,"-Xq","-v","ON_ERROR_STOP=1","-c",sql],{stdio:"pipe"});
async function port(){
  const socket=createServer();await new Promise(r=>socket.listen(0,"127.0.0.1",r));
  const port=socket.address().port;await new Promise(r=>socket.close(r));return port;
}
async function natsProxy(target){
  const sockets=new Set();let accepted=0;
  const server=createServer(front=>{
    const back=connect(Number(target.port)||4222,target.hostname);
    accepted++;sockets.add(front);front.pipe(back).pipe(front);
    front.on("error",()=>back.destroy());back.on("error",()=>front.destroy());
    front.on("close",()=>{sockets.delete(front);back.destroy()});back.on("close",()=>front.destroy());
  });
  await new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));
  const url=new URL(target);url.hostname="127.0.0.1";url.port=String(server.address().port);
  return {url:String(url),sockets,total:()=>accepted,async closed(){for(let i=0;i<100&&sockets.size;i++)await delay(20);assert.equal(sockets.size,0);},async stop(){for(const s of sockets)s.destroy();await new Promise(resolve=>server.close(resolve))}};
}
for(const broker of ["postgres",...(process.env.NATS_URL?["nats"]:[])]){
  const folder=await mkdtemp(join(tmpdir(),"paydirt-replicas-"));
  const schema="pd_replica_"+randomBytes(8).toString("hex");
  const servers=[];
  const proxy=broker==="nats"?await natsProxy(new URL(process.env.NATS_URL)):null;
  try{
    ddl("CREATE SCHEMA "+schema);
    const url=new URL(process.env.DATABASE_URL);url.searchParams.set("options","-csearch_path="+schema);
    const actor=Buffer.from(id);actor[9]=2;
    const other=Buffer.from(id);other[15]^=1;
    const member={tenant:id.toString("hex"),actor:actor.toString("hex"),topics:{read:["n:jobs"],publish:["*"],producer:"worker"}};
    const policy=join(folder,"topic-policy");await writeFile(policy,"allowed");
    const env={...process.env,FLEET_TEST_TOPIC_POLICY:policy,DATABASE_URL:url.href,PAYDIRT_PRINCIPALS:JSON.stringify({member,other:other.toString("hex")})};
    const watch=broker==="nats"?{broker,dsn:proxy.url,namespace:schema}:{broker};
    await writeFile(join(folder,"app.json"),JSON.stringify({listen:"127.0.0.1:0",service_name:"replicas",allow_general_writes:true,watch}));
    await promisify(execFile)(app,["app.Tenant","Add",JSON.stringify({id:id.toString("base64"),alias:"replica"})],{cwd:folder,env});
    await promisify(execFile)(app,["app.Tenant","Add",JSON.stringify({id:other.toString("base64"),alias:"elsewhere"})],{cwd:folder,env});
    const clients=[],transports=[];
    for(let i=0;i<2;i++){
      const addr="127.0.0.1:"+await port();
      const child=spawn(app,[],{cwd:folder,env:{...env,PAYDIRT_LISTEN:addr},stdio:["ignore","ignore","inherit"]});
      const done=new Promise(resolve=>child.once("exit",resolve));servers.push({child,done});
      let ready=false;
      for(let n=0;n<150&&child.exitCode===null;n++){try{await fetch("http://"+addr);ready=true;break;}catch{await delay(30)}}
      assert(ready);
      const transport=createConnectTransport({baseUrl:"http://"+addr,useBinaryFormat:true});
      transports.push(transport);clients.push(createClient(registry.getService("app.RobotService"),transport));
    }
    const row=await clients[1].add({tenant:reference(id),alias:"arm"},opts);
    const ref=reference(row.id);
    const stream=clients[0].watch({filters:[{ref}]},{...opts,timeoutMs:5000})[Symbol.asyncIterator]();
    try{
      assert.equal((await stream.next()).value.items[0].value.alias,"arm");
      await clients[1].patch({ref,alias:"remote-change",dateUpdated:row.dateUpdated},opts);
      assert.equal((await stream.next()).value.items[0].value.alias,"remote-change");
    }finally{await stream.return?.()}
    const svc=(name,i)=>createClient(registry.getService(name),transports[i]);
    const holder=svc("app.HolderService",0),remoteHolder=svc("app.HolderService",1);
    const h=await remoteHolder.add({tenant:reference(id),alias:"bob"},opts);
    const holderAbort=new AbortController(),tracking=holder.watch({filters:[{ref:reference(h.id)}]},{...opts,signal:holderAbort.signal,timeoutMs:5000})[Symbol.asyncIterator]();
    try{
      assert.equal((await tracking.next()).value.items[0].value.alias,"bob");
      await remoteHolder.erase(reference(h.id),opts);
      const item=(await tracking.next()).value.items[0];
      assert.deepEqual(item.id,h.id);assert(!item.value);assert.equal(item.action,"/app.HolderService/Erase");
    }finally{holderAbort.abort();await tracking.return?.()}
    const forbidden=await clients[1].add({alias:"hidden",tenant:reference(other)},{headers:{authorization:"Bearer other"}});
    await assert.rejects(clients[0].watch({filters:[{ref:reference(forbidden.id)}]},{...opts,timeoutMs:4000})[Symbol.asyncIterator]().next(),e=>e.code===5);
    const inbox=svc("app.NotificationService",0),publisher=svc("app.NotificationService",1);
    const topic={key:{case:"name",value:"jobs"}},req={tenantId:id,topics:[topic]};
    const stop=new AbortController(),feed=inbox.listen(req,{...opts,signal:stop.signal,timeoutMs:8000})[Symbol.asyncIterator]();
    try{
      assert.equal((await feed.next()).value.event.case,"ready");
      const notice=await publisher.add({tenantId:id,topic,kind:"work.done",idempotencyKey:"cross-replica"},opts);
      assert.equal((await feed.next()).value.event.case,"invalidated");
      const page=await inbox.read(req,opts);assert.deepEqual(page.items[0].notification.id,notice.id);
      await publisher.mark({tenantId:id,refs:[{id:notice.id}]},opts);
      assert.equal((await feed.next()).value.event.case,"invalidated");
      assert.equal((await inbox.count({...req,filter:{unreadOnly:true}},opts)).count,0n);
      await writeFile(policy,"revoked");
      await assert.rejects(feed.next(),e=>e.code===7);
      await assert.rejects(inbox.get({ref:{id:notice.id}},opts),e=>e.code===5);
      await writeFile(policy,"allowed");
    }finally{stop.abort();await feed.return?.()}
    const stopResources=new AbortController();let onHeader;
    const ready=new Promise(resolve=>onHeader=resolve);
    const resources=svc("payday.ResourceService",0).watch({},{...opts,onHeader,signal:stopResources.signal,timeoutMs:10000})[Symbol.asyncIterator]();
    const receiving=resources.next();receiving.catch(()=>{});
    try{
      await Promise.race([ready,receiving.then(()=>{throw Error("unexpected initial resource")})]);
      const large="remote-payload-".repeat(2400);
      const entry=await svc("shared.ThingService",1).add({alias:"large-thing",description:large},opts);
      const e=(await receiving).value;
      assert.equal(e.method,"/shared.ThingService/Add");assert.equal(e.callMethod,e.method);
      const stored=fromBinary(registry.getMessage("shared.Thing"),e.value.value);
      assert.deepEqual(entry.id,stored.id);assert.equal(stored.description,large);
    }finally{stopResources.abort();await resources.return?.()}
    if(proxy){
      assert(proxy.sockets.size>=2);
      for(const {child} of servers)child.kill("SIGTERM");
      await Promise.all(servers.map(s=>s.done));await proxy.closed();
      const before=proxy.total();
      const bad=spawn(app,[],{cwd:folder,env:{...env,PAYDIRT_POLICY_FILE:join(folder,"missing-policy")},stdio:"ignore"});
      const code=await new Promise(resolve=>bad.once("exit",resolve));assert.notEqual(code,0);assert(proxy.total()>before);await proxy.closed();
    }
    console.log(broker,"delivered a committed update between independent app processes");
  }finally{
    for(const {child} of servers)child.kill();
    await Promise.all(servers.map(s=>s.done));ddl("DROP SCHEMA IF EXISTS "+schema+" CASCADE");
    if(proxy)await proxy.stop();
    await rm(folder,{recursive:true,force:true});
  }
}

async function checkWorkerDiscovery(){
  const folder=await mkdtemp(join(tmpdir(),"paydirt-workers-")),db=join(folder,"store.sqlite");
  const env={...process.env,DATABASE_URL:db,PAYDIRT_PRINCIPALS:JSON.stringify({member:id.toString("hex")})};
  const address="127.0.0.1:"+await port();
  const configure=watch=>writeFile(join(folder,"app.json"),JSON.stringify({listen:address,service_name:"workers",allow_general_writes:true,watch}));
  const tables=JSON.parse(await readFile(join(project,"generated/storage/model.json"),"utf8")).entities;
  const date="f"+tables.find(e=>e.name==="app.NotificationRecord").fields.find(f=>f.name==="date_created").number;
  const sql=statement=>JSON.parse(execFileSync("python3",["-c",`import sqlite3,sys,json
with sqlite3.connect(sys.argv[1]) as db:
    print(json.dumps(db.execute(sys.argv[2]).fetchall()))`,db,statement],{encoding:"utf8"}));
  let processHandle,exited;
  const start=async()=>{
    processHandle=spawn(app,[],{env,cwd:folder,stdio:"ignore"});
    exited=new Promise(resolve=>processHandle.once("exit",resolve));
    for(let i=0;i<100;i++){if(processHandle.exitCode!==null)throw Error("worker server exited");try{await fetch("http://"+address);return;}catch{await delay(30)}}
    throw Error("worker server did not start");
  };
  const stop=async()=>{if(processHandle){processHandle.kill("SIGTERM");await exited;processHandle=undefined;}};
  const invoke=(entity,method,arg)=>JSON.parse(execFileSync(app,[entity,method,JSON.stringify(arg)],{env,cwd:folder,encoding:"utf8"}));
  try{
    await configure({});
    await assert.rejects(promisify(execFile)(app,[],{cwd:folder,env,timeout:5000}),e=>e.stderr.includes("watch.broker"));
    await configure({broker:"memory",outbox:true});
    invoke("app.Tenant","Add",{id:id.toString("base64"),alias:"jobs"});
    const robot=invoke("app.Robot","Add",{tenant:{id:id.toString("base64")},alias:"visible"});
    invoke("app.Notification","Add",{tenantId:id.toString("base64"),topic:{name:"jobs"},kind:"done",idempotencyKey:"old"});
    sql('UPDATE "pd_app__NotificationRecord" SET '+date+"='2000-01-01T00:00:00Z'");
    const count=table=>sql('SELECT count(*) FROM "pd_app__'+table+'"')[0][0];
    assert.equal(count("NotificationRecord"),1);assert(count("Outbox")>0);
    await start();
    for(let i=0;i<120 && (count("NotificationRecord")||count("Outbox"));i++)await delay(50);
    assert.equal(count("NotificationRecord"),0);assert.equal(count("Outbox"),0);await stop();
    await configure({broker:"none"});await start();
    const service=createClient(registry.getService("app.RobotService"),createConnectTransport({baseUrl:"http://"+address,useBinaryFormat:true}));
    const stream=service.watch({filters:[{ref:reference(Buffer.from(robot.id,"base64"))}]},{...opts,timeoutMs:3000});
    await assert.rejects(stream[Symbol.asyncIterator]().next(),e=>e.code===12 && e.message.includes("nothing to watch"));
  }finally{await stop();await rm(folder,{recursive:true,force:true});}
}
await checkWorkerDiscovery();
