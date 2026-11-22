import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { spawn,execFileSync } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { createServer } from "vite";
import { createClient,Code } from "@connectrpc/connect";
import { createConnectTransport } from "@connectrpc/connect-web";

import {operatorScenario} from "./generated-operator.mjs";
import { exerciseTenancy } from "./generated-tenancy.mjs";
import {queryScenario} from "./generated-query.js";
import {goReference} from "./compare-queries.mjs";
import {filePolicy} from "./generated-file-policy.mjs";
import {accounts,accountCases} from "./generated-account.js";
import {pastScenario} from "./generated-past.js";
import {runPair} from "./generated-pair.js";
import {fromBinary,createFileRegistry} from "@bufbuild/protobuf";
import {FileDescriptorSetSchema} from "@bufbuild/protobuf/wkt";
import {readFile,writeFile} from "node:fs/promises";
import { exerciseMove } from "./generated-move.js";
const app = process.env.GENERATED_APP;
const [primary,other] = Array.from({length:2},()=>{
  const id=randomBytes(16); id[0]=172; id[6]=(id[6]&15)|128; id[8]=(id[8]&63)|128; id[9]=1;
  return {id, token:randomBytes(24).toString("hex")};
});
const clients = [primary,other];
const sandbox=await createServer({server:{middlewareMode:true},appType:"custom"});
const services=await sandbox.ssrLoadModule("/upstream/internal/apptest/ts/gen/app/robot_svc_pb.ts");
const {CellService, RobotService}=services;
const schema="pd_accept_"+randomBytes(10).toString("hex");
const ddl=sql=>execFileSync("psql",[process.env.DATABASE_URL,"-Xq","-v","ON_ERROR_STOP=1","-c",sql],{stdio:"pipe"});
const tenantRef=id=>({key:{case:"id",value:id}});
const opts=token=>({headers:{authorization:"Bearer "+token}});
const reference=process.env.PAYDIRT_TEST_MOVE==="1" && process.env.PAYDIRT_GO_REFERENCE ? await goReference(sandbox) : undefined;
if(reference)await writeFile(process.env.CELL_PROJECT+"/query-reference.json",JSON.stringify(reference));
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
    const allowedSet=Buffer.from(primary.id);allowedSet[9]=10;
    principals.both={tenant:primary.id.toString("hex"),tenants:clients.map(p=>p.id.toString("hex"))};
    principals.grouped={tenant:primary.id.toString("hex"),sets:[allowedSet.toString("hex")],methods:["/app.*/*"]};
    principals.reader={tenant:primary.id.toString("hex"),methods:["/app.RobotService/Get"]};
    const tenantNames=clients.map(p=>p.id.toString("hex"));
    const membership=accounts(tenantNames);
    if(process.env.PAYDIRT_TEST_MOVE==="1") Object.assign(principals,membership.principals);
    const env={...process.env,DATABASE_URL:url,PAYDIRT_PRINCIPALS:JSON.stringify(principals),PAYDIRT_LISTEN:"127.0.0.1:"+process.env.CELL_PORT};
    for(const {id} of clients)execFileSync(app,["app.Tenant","Add",JSON.stringify({id:id.toString("base64"),alias:id.toString("hex")})],{cwd:process.env.CELL_PROJECT,env});
    if(process.env.PAYDIRT_TEST_MOVE==="1"){
      for(const seed of membership.seeds) execFileSync(app,[seed.entity,"Add",JSON.stringify(seed.row)],{cwd:process.env.CELL_PROJECT,env});
    }
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
        const descriptors=fromBinary(FileDescriptorSetSchema,await readFile(process.env.CELL_PROJECT+"/generated/storage/descriptors.binpb"));
        const robotType=createFileRegistry(descriptors).getService("app.RobotService");
        const typed=createClient(robotType,createConnectTransport({baseUrl,useBinaryFormat:true}));
        const holder=createClient(createFileRegistry(descriptors).getService("app.HolderService"),createConnectTransport({baseUrl,useBinaryFormat:true}));
        await accountCases(holder,typed,tenantNames);
        await runPair(typed,own,primary.id);
        for(const [table,column] of [["Audit","f10"],["Outbox","f9"]]){
          const sql='SELECT count(*) FROM "pd_app__'+table+'" WHERE '+column+"='/app.RobotService/RenamePair'";
          const count= engine==="postgres" ? execFileSync("psql",[url,"-Atq","-c",sql],{encoding:"utf8"}).trim()
            : execFileSync("python3",["-c","import sqlite3,sys;print(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchone()[0])",url,sql],{encoding:"utf8"}).trim();
          assert.equal(count,"2",table+" must contain exactly the committed pair");
        }
        const report=await queryScenario(robots,primary.id,own);
        const owned=name=>createClient(createFileRegistry(descriptors).getService("app."+name+"Service"),createConnectTransport({baseUrl,useBinaryFormat:true}));
        report.past=await pastScenario({robot:typed,cell,event:owned("Event"),fact:owned("Fact")},primary.id,own);
        if(reference){assert.deepEqual(report,reference);console.log(engine,"matches the Go reference for List and Watch",report);}
        await exerciseMove(robots,cell,own,foreign,opts("represented"),opts("reader"),primary.id,other.id);
        await exerciseTenancy(services,createConnectTransport({baseUrl,useBinaryFormat:true}),primary,other,allowedSet);
        await operatorScenario(app,process.env.CELL_PROJECT,{...env,PAYDIRT_URL:baseUrl,PAYDIRT_TOKEN:primary.token},primary);
        console.log(engine,"served the user-written Move and enforced the dispatched method grant");
      }
      console.log(engine,"served Cell CRUD and enforced tenant scope using only the app entity declaration");
    } finally {server.kill();await exited}
    if(process.env.PAYDIRT_TEST_MOVE==="1"){
      const descriptors=fromBinary(FileDescriptorSetSchema,await readFile(process.env.CELL_PROJECT+"/generated/storage/descriptors.binpb"));
      await filePolicy(app,process.env.CELL_PROJECT,env,membership,tenantNames,createFileRegistry(descriptors));
      console.log(engine,"resolved account membership, enforced file roles and revoked an open stream");
    }
  }
} finally {
  if (created) ddl("DROP SCHEMA "+schema+" CASCADE");
  await sandbox.close();
}
