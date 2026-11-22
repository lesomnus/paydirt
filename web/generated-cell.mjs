import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { spawn,execFileSync } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { createServer } from "vite";
import { createClient,Code } from "@connectrpc/connect";
import { createConnectTransport } from "@connectrpc/connect-web";

import { exerciseMove } from "./generated-move.js";
const app = process.env.GENERATED_APP;
const [primary,other] = Array.from({length:2},()=>{
  const id=randomBytes(16); id[6]=(id[6]&15)|128; id[8]=(id[8]&63)|128; id[9]=1;
  return {id, token:randomBytes(24).toString("hex")};
});
const clients = [primary,other];
const sandbox=await createServer({server:{middlewareMode:true},appType:"custom"});
const { CellService, RobotService }=await sandbox.ssrLoadModule("/upstream/internal/apptest/ts/gen/app/robot_svc_pb.ts");
const schema="pd_accept_"+randomBytes(10).toString("hex");
const ddl=sql=>execFileSync("psql",[process.env.DATABASE_URL,"-Xq","-v","ON_ERROR_STOP=1","-c",sql],{stdio:"pipe"});
const tenantRef=id=>({key:{case:"id",value:id}});
const opts=token=>({headers:{authorization:"Bearer "+token}});
let created = false;
try {
  for(const engine of ["postgres","sqlite"]){
    let url=process.env.CELL_SQLITE_PATH;
    if(engine==="postgres"){
      ddl("CREATE SCHEMA "+schema);
      created = true;
      const pg=new URL(process.env.DATABASE_URL);pg.searchParams.set("options","-csearch_path="+schema);url=pg.href;
    }
    const principals=Object.fromEntries(clients.map(({token,id})=>[token,id.toString("hex")]));
    const actor=Buffer.from(primary.id);actor[9]=2;
    principals.represented={tenant:primary.id.toString("hex"),actor:actor.toString("hex"),caller:actor.toString("hex"),caller_tenant:primary.id.toString("hex")};
    principals.reader={tenant:primary.id.toString("hex"),methods:["/app.RobotService/Get"]};
    const env={...process.env,DATABASE_URL:url,PAYDIRT_PRINCIPALS:JSON.stringify(principals),PAYDIRT_LISTEN:"127.0.0.1:"+process.env.CELL_PORT};
    for(const {id} of clients)execFileSync(app,["app.Tenant","Add",JSON.stringify({id:id.toString("base64"),alias:id.toString("hex")})],{cwd:process.env.CELL_PROJECT,env});
    const server=spawn(app,[],{cwd:process.env.CELL_PROJECT,env,stdio:"inherit"});
    const exited=new Promise(resolve=>server.once("exit",resolve));
    try{
      const baseUrl="http://"+env.PAYDIRT_LISTEN;
      let up=false; for(let i=0;i<150&&server.exitCode===null;i++){try{await fetch(baseUrl);up=true;break}catch{await delay(50)}}
      assert(up,"generated server did not start");
      const cell=createClient(CellService,createConnectTransport({baseUrl,useBinaryFormat:true}));
      const own=opts(primary.token), foreign=opts(other.token);
      const row=await cell.add({tenant:tenantRef(primary.id),alias:"dock"},own);
      assert.equal(row.id[9],10); const ref=tenantRef(row.id);
      assert.equal((await cell.get({ref},own)).alias,"dock");
      await assert.rejects(cell.get({ref},foreign),e=>e.code===Code.NotFound);
      await assert.rejects(cell.patch({ref,alias:"hidden"},foreign),e=>e.code===Code.NotFound);
      assert.equal((await cell.erase(ref,foreign)).erased,false);
      await assert.rejects(cell.add({tenant:tenantRef(other.id),alias:"foreign"},own),e=>e.code===Code.NotFound);
      assert.equal((await cell.patch({ref,alias:"renamed"},own)).alias,"renamed");
      assert.equal((await cell.erase(ref,own)).erased,true);
      await assert.rejects(cell.get({ref},own),e=>e.code===Code.NotFound);
      if(process.env.PAYDIRT_TEST_MOVE==="1"){
        const robots=createClient(RobotService,createConnectTransport({baseUrl,useBinaryFormat:true}));
        await exerciseMove(robots,cell,own,foreign,opts("represented"),opts("reader"),primary.id,other.id);
        console.log(engine,"served the user-written Move and enforced the dispatched method grant");
      }
      console.log(engine,"served Cell CRUD and enforced tenant scope using only the app entity declaration");
    } finally {server.kill();await exited}
  }
} finally {
  if (created) ddl("DROP SCHEMA "+schema+" CASCADE");
  await sandbox.close();
}
