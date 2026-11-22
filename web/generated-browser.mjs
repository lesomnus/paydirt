import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { cp,writeFile } from "node:fs/promises";
import { join,resolve } from "node:path";
import { createServer } from "vite";
import { chromium } from "playwright";

const root=join(process.env.CELL_PROJECT,"sandbox");
await cp(resolve("upstream/internal/apptest/ts/gen"),join(root,"test-client"),{recursive:true});
await cp(resolve("generated-move.js"),join(root,"moves.js"));
await writeFile(join(root,"harness.js"),`
import { openSandbox } from "./client.js";
import { CellService, RobotService } from "./test-client/app/robot_svc_pb.ts";
import {createClient} from "@connectrpc/connect";
export async function open(principals){
  const sandbox=await openSandbox(principals,"paydirt",{allowGeneralWrites:true});
  return {robots:createClient(RobotService,sandbox.link.transport),client:createClient(CellService,sandbox.link.transport),close:()=>sandbox.close()};
}
`);
const server=await createServer({root,optimizeDeps:{entries:["index.html","harness.js"]},server:{hmr:false,host:"127.0.0.1",port:0},logLevel:"error"});
let browser;
try{
  await server.listen();
  browser=await chromium.launch({executablePath:process.env.CHROMIUM_PATH});
  const page=await browser.newPage();
  page.on("pageerror",e=>console.error(e));
  const tenants=Object.fromEntries(Array.from({length:2},()=>{
    const b=randomBytes(16);b[6]=128;b[8]=128;b[9]=1;return [randomBytes(16).toString("hex"),b.toString("hex")];
  }));
  await page.goto(server.resolvedUrls.local[0]);
  await page.locator("#read").click();
  await page.waitForFunction(()=>document.querySelector("#result")?.textContent==="paydirt",{},{timeout:20000});
  await page.reload();
  const [[first,tenant],[second]]=Object.entries(tenants);
  const actor=Buffer.from(tenant,"hex");actor[9]=2;
  tenants.represented={tenant,actor:actor.toString("hex"),caller:actor.toString("hex"),caller_tenant:tenant};
  tenants.reader={tenant,methods:["/app.RobotService/Get"]};
  const check=async({tenants,id,move})=>{
    const {open}=await import("/harness.js");const app=await open(tenants);
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
        const {exerciseMove}=await import("/moves.js");
        const otherBytes=Uint8Array.from(other.match(/../g),s=>parseInt(s,16));
        return {cell:Array.from(cell.id),...await exerciseMove(app.robots,app.client,opts(a),opts(b),opts("represented"),opts("reader"),bytes,otherBytes)};
      }
      return {cell:Array.from(cell.id)};
    }finally{app.close()}
  };
  const id=await page.evaluate(check,{tenants,move:process.env.PAYDIRT_TEST_MOVE==="1"});
  assert.equal(id.cell[9],10);
  await page.reload();
  assert.equal(await page.evaluate(check,{tenants,id}),"persisted");
  console.log("Generated app served upstream CellService on dRPC and reloaded its SQLite row after both Workers restarted.");
}finally{await browser?.close();await server.close()}
