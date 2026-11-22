import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { cp,readFile,writeFile } from "node:fs/promises";
import { join,resolve } from "node:path";
import { createServer } from "vite";
import { chromium } from "playwright";

const root=join(process.env.CELL_PROJECT,"sandbox");
await cp(resolve("upstream/internal/apptest/ts/gen"),join(root,"test-client"),{recursive:true});
await cp(resolve("generated-notify.js"),join(root,"notify.js"));
await cp(resolve("generated-past.js"),join(root,"past.js"));
await cp(resolve("generated-resource.js"),join(root,"resource.js"));
await cp(resolve("generated-account.js"),join(root,"account.js"));
await cp(resolve("generated-readings.js"),join(root,"readings.js"));
await cp(resolve("generated-request-streams.js"),join(root,"request-streams.js"));
await cp(resolve("generated-pair.js"),join(root,"pair.js"));
await cp(resolve("generated-query.js"),join(root,"query.js"));
await cp(resolve("generated-move.js"),join(root,"moves.js"));
await writeFile(join(root,"harness.js"),`
import { openSandbox } from "./client.js";
import { fromBinary, createFileRegistry } from "@bufbuild/protobuf";
import {FileDescriptorSetSchema} from "@bufbuild/protobuf/wkt";
import { CellService, RobotService } from "./test-client/app/robot_svc_pb.ts";
import {createClient} from "@connectrpc/connect";
import {accounts} from "./account.js";
export async function open(principals,withAccounts=false,restricted=false){
  const membership=accounts(Object.values(principals).slice(0,2));
  const sandbox=await openSandbox(withAccounts?{...principals,...membership.principals}:principals,"paydirt",{allowGeneralWrites:true,seeds:withAccounts?[...membership.seeds,membership.grant]:[],policy:restricted?membership.policy:undefined});
  const bytes=new Uint8Array(await (await fetch("./pkg/descriptors.binpb")).arrayBuffer());
  const registry=createFileRegistry(fromBinary(FileDescriptorSetSchema,bytes));const robot=registry.getService("app.RobotService");const holder=registry.getService("app.HolderService");
  const owned=name=>{const svc=registry.getService("app."+name+"Service");return svc && createClient(svc,sandbox.link.transport)};return {registry,resources:createClient(registry.getService("payday.ResourceService"),sandbox.link.transport),robotService:robot,readings:owned("ComputedReading"),notification:owned("Notification"),accountNotification:owned("NotificationAccount"),event:owned("Event"),fact:owned("Fact"),holder:holder && createClient(holder,sandbox.link.transport),typed:robot && createClient(robot,sandbox.link.transport),robots:createClient(RobotService,sandbox.link.transport),client:createClient(CellService,sandbox.link.transport),close:()=>sandbox.close()};
}
`);
const server=await createServer({root,optimizeDeps:{entries:["index.html","harness.js","request-streams.js"],exclude:["@lesomnus/grpc-dgram"]},server:{hmr:false,host:"127.0.0.1",port:0},logLevel:"error"});
let browser;
try{
  await server.listen();
  browser=await chromium.launch({executablePath:process.env.CHROMIUM_PATH});
  const page=await browser.newPage();
  page.on("pageerror",e=>console.error(e));
  const tenants=Object.fromEntries(Array.from({length:2},()=>{
    const b=randomBytes(16);b[0]=172;b[6]=128;b[8]=128;b[9]=1;return [randomBytes(16).toString("hex"),b.toString("hex")];
  }));
  await page.goto(server.resolvedUrls.local[0]);
  await page.locator("#read").click();
  await page.waitForFunction(()=>document.querySelector("#result")?.textContent==="paydirt",{},{timeout:20000});
  await page.reload();
  const [[first,tenant],[second]]=Object.entries(tenants);
  const actor=Buffer.from(tenant,"hex");actor[9]=2;
  tenants.represented={tenant,actor:actor.toString("hex"),caller:actor.toString("hex"),caller_tenant:tenant};
  tenants.reader={tenant,methods:["/app.RobotService/Get"]};
  const check=async({tenants,id,move,compareGo})=>{
    const {open}=await import("/harness.js");const app=await open(tenants,move);
    const [[a,tenant],[b,other]]=Object.entries(tenants);
    const opts=token=>({headers:{authorization:"Bearer "+token}});
    const key=b=>({key:{case:"id",value:b}});
    try{
      if(id){
        if(id.robot){
          const found=await app.robots.get({ref:key(Uint8Array.from(id.robot))},opts(a));
          if(found.cell?.id.join()!==id.destination.join())throw Error("Move lost on Worker restart");
        }
        return (await app.client.get({ref:key(Uint8Array.from(id.cell))},opts(a))).alias;
      }
      const bytes=Uint8Array.from(tenant.match(/../g),s=>parseInt(s,16));
      const cell=await app.client.add({tenant:key(bytes),alias:"dock"},opts(a));
      const ref=key(cell.id);
      await app.client.patch({ref,alias:"persisted"},opts(a));
      let code=0;
      try {await app.client.get({ref},opts(b))}catch(e){code=e.code}
      if(code!==5)throw Error("another tenant could reach the Cell");
      if(move){
        const {accountCases}=await import("/account.js");await accountCases(app.holder,app.typed,[tenant,other]);
        const {checkReadings}=await import("/readings.js");
        await checkReadings(app.readings);
        const {requestStreams}=await import("/request-streams.js");await requestStreams(app.typed,app.robotService,opts(a),compareGo);
        const {runPair}=await import("/pair.js");
        await runPair(app.typed,opts(a),bytes);
        const {resourceCases}=await import("/resource.js");
        await resourceCases(app.registry,app.resources,app.cell,app.typed,opts(a),opts(b),bytes,Uint8Array.from(tenants[b].match(/../g),v=>parseInt(v,16)),false);

        if(!(await app.typed.allowance({key:"fleet.browser",lock:true},opts(a))).allowsTwo || (await app.typed.allowance({key:"fleet.browser",lock:true},opts(b))).enabled)throw Error("entitlement tenant or limit mismatch");
        const {exerciseMove}=await import("/moves.js");
        const {queryScenario}=await import("/query.js");
        const report=await queryScenario(app.robots,bytes,opts(a));
        const {notificationCases}=await import("/notify.js");const actor=bytes.slice();actor[9]=2;
        await notificationCases(app.notification,app.accountNotification,bytes,actor,opts);
        const {pastScenario}=await import("/past.js"); report.past=await pastScenario({robot:app.robots,cell:app.client,event:app.event,fact:app.fact},bytes,opts(a));
        const otherBytes=Uint8Array.from(other.match(/../g),s=>parseInt(s,16));
        return {report,cell:Array.from(cell.id),...await exerciseMove(app.robots,app.client,opts(a),opts(b),opts("represented"),opts("reader"),bytes,otherBytes)};
      }
      return {cell:Array.from(cell.id)};
    }finally{app.close()}
  };
  const id=await page.evaluate(check,{tenants,compareGo:!!process.env.PAYDIRT_GO_CHECKOUT,move:process.env.PAYDIRT_TEST_MOVE==="1"});
  assert.equal(id.cell[9],10);
  if(id.report && process.env.PAYDIRT_GO_REFERENCE){
    assert.deepEqual(id.report,JSON.parse(await readFile(join(process.env.CELL_PROJECT,"query-reference.json"),"utf8")));
    console.log("Browser dRPC List and Watch match the Go reference");
  }
  if(process.env.PAYDIRT_TEST_MOVE==="1"){
    console.log(`Client streaming, live bidi, half-close and cancellation passed on ${process.env.PAYDIRT_GO_CHECKOUT ? "Go and Rust" : "Rust"} dRPC Workers.`);
    await page.evaluate(async tenants=>{
      const {open}=await import("/harness.js"),{accountCases}=await import("/account.js");
      const app=await open(tenants,true,true);
      try{await accountCases(app.holder,app.typed,Object.values(tenants).slice(0,2),true)}finally{app.close()}
    },tenants);
  }
  await page.reload();
  assert.equal(await page.evaluate(check,{tenants,id}),"persisted");
  console.log("Generated app served upstream CellService on dRPC and reloaded its SQLite row after both Workers restarted.");
}finally{await browser?.close();await server.close()}
