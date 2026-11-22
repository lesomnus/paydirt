import assert from "node:assert/strict";
import {execFileSync,spawn} from "node:child_process";
import {randomBytes} from "node:crypto";
import {mkdtemp,readFile,rm,writeFile} from "node:fs/promises";
import {createServer} from "node:http";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {setTimeout as delay} from "node:timers/promises";
import {createClient,Code,ConnectError} from "@connectrpc/connect";
import {connectNodeAdapter} from "@connectrpc/connect-node";
import {createConnectTransport} from "@connectrpc/connect-web";
import {createFileRegistry,fromBinary} from "@bufbuild/protobuf";
import {FileDescriptorSetSchema} from "@bufbuild/protobuf/wkt";

const project=process.env.CELL_PROJECT, binary=process.env.GENERATED_APP;
const registry=createFileRegistry(fromBinary(FileDescriptorSetSchema,await readFile(join(project,"generated/storage/descriptors.binpb"))));
const grantService=registry.getService("payday.GrantService"), robotService=registry.getService("app.RobotService");
const tenant=randomBytes(16);tenant[0]=172;tenant[6]=128;tenant[8]=128;tenant[9]=1;
const key="fleet.followed";
const grant=(version,limit)=>({id:Uint8Array.of(7),tenantId:tenant,key,version:BigInt(version),value:{case:"limit",value:BigInt(limit)},dateExpires:{seconds:4070908800n,nanos:0}});
let watchCount=0, failPage=true, secondPages=0, releaseWatch=false, reconnect=false;
let version=1,limit=5,announced=0;
function authorized(ctx){if(ctx.requestHeader.get("authorization")!=="Bearer source-secret")throw new ConnectError("token",Code.Unauthenticated);}
const handler=connectNodeAdapter({routes(router){router.service(grantService,{
  async *watch(_,ctx){
    authorized(ctx);const n=++watchCount;
    if(n===1)throw new ConnectError("starting",Code.Unavailable);
    if(n===2){yield {event:{case:"grant",value:grant(100,100)}};return;}
    if(n>3 && !reconnect)throw new ConnectError("offline",Code.Unavailable);
    yield {event:{case:"ready",value:{}}};
    while(!ctx.signal.aborted){
      if(n===3 && releaseWatch)return;
      if(announced){const value=announced;announced=0;yield {event:{case:"grant",value:grant(value,1)}};}
      await delay(8);
    }
  },
  async list(req,ctx){
    authorized(ctx);assert(watchCount>=3,"listed before ready");
    if(req.cursor==="")return {items:[grant(version,limit)],next:"more"};
    assert.equal(req.cursor,"more");secondPages++;
    if(failPage)throw new ConnectError("incomplete page",Code.Unavailable);
    return {};
  }
});}});
const source=createServer(handler);
await new Promise(resolve=>source.listen(0,"127.0.0.1",resolve));
const dir=await mkdtemp(join(tmpdir(),"grant-follow-"));
const listen="127.0.0.1:"+process.env.CELL_PORT;
const env={...process.env,DATABASE_URL:join(dir,"grant.db"),PAYDIRT_LISTEN:listen,PAYDIRT_PRINCIPALS:JSON.stringify({app:tenant.toString("hex")}),PAYDIRT_GRANT_TEST_TOKEN:"source-secret"};
const cfg=JSON.parse(await readFile(join(project,"app.json"),"utf8"));
cfg.grant_source={url:"http://127.0.0.1:"+source.address().port,bearer_env:"PAYDIRT_GRANT_TEST_TOKEN",every_ms:100,retry_ms:30};
await writeFile(join(dir,"app.json"),JSON.stringify(cfg));
const options={headers:{authorization:"Bearer app"},timeoutMs:500};
const robot=createClient(robotService,createConnectTransport({baseUrl:"http://"+listen,useBinaryFormat:true}));
const quota=()=>robot.allowance({key,lock:true},options);
async function until(test,msg){
  const end=Date.now()+10000;let last;
  while(Date.now()<end){try{if(await test())return;}catch(e){last=e;}await delay(20);}
  throw new Error(msg,{cause:last});
}
let server,exit;
try{
  execFileSync(binary,["app.Tenant","Add",JSON.stringify({id:tenant.toString("base64"),alias:"billing"})],{cwd:dir,env});
  server=spawn(binary,[],{cwd:dir,env,stdio:"inherit"});exit=new Promise(r=>server.once("exit",r));
  await until(()=>secondPages>0,"source never reached page two");
  assert.equal((await quota()).enabled,false,"partial snapshot changed grant");
  failPage=false;
  await until(async()=>(await quota()).allowsTwo,"complete snapshot did not arrive");
  announced=2;
  await until(async()=>!(await quota()).allowsTwo,"stream update did not arrive");
  assert.equal((await quota()).enabled,true);
  releaseWatch=true;
  await until(()=>watchCount>=4,"follower never reconnected");
  assert.equal((await quota()).allowsTwo,false);
  version=3;limit=8;reconnect=true;
  await until(async()=>(await quota()).allowsTwo,"follower did not reconcile after reconnect");
  console.log("External GrantService initial failure, ready, partial snapshot, update and reconnect passed.");
}finally{
  if(server){server.kill();await exit;}
  await new Promise(resolve=>source.close(resolve));await rm(dir,{recursive:true,force:true});
}
