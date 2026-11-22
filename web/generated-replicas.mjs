// Run two independent generated app servers on one tenant's database.
import assert from "node:assert/strict";
import {randomBytes} from "node:crypto";
import {spawn,execFileSync} from "node:child_process";
import {mkdtemp,readFile,writeFile,rm} from "node:fs/promises";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {createServer} from "node:net";
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
for(const broker of ["postgres",...(process.env.NATS_URL?["nats"]:[])]){
  const folder=await mkdtemp(join(tmpdir(),"paydirt-replicas-"));
  const schema="pd_replica_"+randomBytes(8).toString("hex");
  const servers=[];
  try{
    ddl("CREATE SCHEMA "+schema);
    const url=new URL(process.env.DATABASE_URL);url.searchParams.set("options","-csearch_path="+schema);
    const env={...process.env,DATABASE_URL:url.href,PAYDIRT_PRINCIPALS:JSON.stringify({member:id.toString("hex")})};
    const watch=broker==="nats"?{broker,dsn:process.env.NATS_URL,namespace:schema}:{broker};
    await writeFile(join(folder,"app.json"),JSON.stringify({listen:"127.0.0.1:0",service_name:"replicas",allow_general_writes:true,watch}));
    execFileSync(app,["app.Tenant","Add",JSON.stringify({id:id.toString("base64"),alias:"replica"})],{cwd:folder,env});
    const clients=[];
    for(let i=0;i<2;i++){
      const addr="127.0.0.1:"+await port();
      const child=spawn(app,[],{cwd:folder,env:{...env,PAYDIRT_LISTEN:addr},stdio:["ignore","ignore","inherit"]});
      const done=new Promise(resolve=>child.once("exit",resolve));servers.push({child,done});
      let ready=false;
      for(let n=0;n<150&&child.exitCode===null;n++){try{await fetch("http://"+addr);ready=true;break;}catch{await delay(30)}}
      assert(ready);
      clients.push(createClient(registry.getService("app.RobotService"),createConnectTransport({baseUrl:"http://"+addr,useBinaryFormat:true})));
    }
    const row=await clients[1].add({tenant:reference(id),alias:"arm"},opts);
    const ref=reference(row.id);
    const stream=clients[0].watch({filters:[{ref}]},{...opts,timeoutMs:5000})[Symbol.asyncIterator]();
    try{
      assert.equal((await stream.next()).value.items[0].value.alias,"arm");
      await clients[1].patch({ref,alias:"remote-change",dateUpdated:row.dateUpdated},opts);
      assert.equal((await stream.next()).value.items[0].value.alias,"remote-change");
    }finally{await stream.return?.()}
    console.log(broker,"delivered a committed update between independent app processes");
  }finally{
    for(const {child} of servers)child.kill();
    await Promise.all(servers.map(s=>s.done));ddl("DROP SCHEMA IF EXISTS "+schema+" CASCADE");
    await rm(folder,{recursive:true,force:true});
  }
}
