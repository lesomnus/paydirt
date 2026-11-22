// Both listeners of a generated server share the application's routes and RPC policies.
import assert from "node:assert/strict";
import {execFileSync,spawn} from "node:child_process";
import {randomBytes} from "node:crypto";
import {once} from "node:events";
import {mkdtemp,rm,writeFile} from "node:fs/promises";
import {connect} from "node:http2";
import {createServer} from "node:net";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {setTimeout as delay} from "node:timers/promises";
const cwd=await mkdtemp(join(tmpdir(),"fleet-routes-"));
const port=async()=>{const s=createServer();s.listen(0,"127.0.0.1");await once(s,"listening");const p=s.address().port;await new Promise(r=>s.close(r));return p;};
const main=await port(),browser=await port(), app=process.env.GENERATED_APP;
const tenant=()=>{const b=randomBytes(16);b[6]=128;b[8]=128;b[9]=1;return b;};
const acme=tenant(),elsewhere=tenant(),env={...process.env,DATABASE_URL:join(cwd,"app.db"),PAYDIRT_LISTEN:"127.0.0.1:"+main,PAYDIRT_PRINCIPALS:JSON.stringify({acme:acme.toString("hex"),elsewhere:elsewhere.toString("hex")})};
let child,status;
const stop=async()=>{if(child){child.kill();await status;child=undefined;}};
const settings={listen:env.PAYDIRT_LISTEN,service_name:"web-contracts",allow_general_writes:true,watch:{broker:"memory"}};
const origin="https://portal.example", route="/app.RobotService/List";
const url=(p,path)=>"http://127.0.0.1:"+p+path;
const call=(p,path,token,body={})=>fetch(url(p,path),{method:"POST",headers:{"content-type":"application/json","connect-protocol-version":"1","authorization":"Bearer "+token},body:JSON.stringify(body)});
async function start(serving) {
  await writeFile(join(cwd,"app.json"),JSON.stringify({...settings,serving}));
  child=spawn(app,[],{cwd,env,stdio:["ignore","ignore","pipe"]});status=once(child,"exit");
  for(let i=0;i<140;i++){try{if((await fetch(url(main,"/teapot"))).status===418)return;}catch{}await delay(30);}
  throw Error("application route was not mounted");
}
async function grpc(port) {
  const client=connect(url(port,""));
  try{
    const req=client.request({":method":"POST",":path":route,"content-type":"application/grpc","te":"trailers","authorization":"Bearer acme"});
    const chunks=[];let headers,trailers;
    req.on("response",h=>headers=h);req.on("trailers",h=>trailers=h);req.on("data",c=>chunks.push(c));req.end(Buffer.alloc(5));await once(req,"end");
    assert.equal(headers[":status"],200);assert.equal(trailers?.["grpc-status"]??headers["grpc-status"],"0");
    assert(Buffer.concat(chunks).includes(Buffer.from("arm-01")));
  }finally{client.close();}
}
try{
  await writeFile(join(cwd,"app.json"),JSON.stringify(settings));
  for(const [id,alias] of [[acme,"acme"],[elsewhere,"elsewhere"]])execFileSync(app,["app.Tenant","Add",JSON.stringify({id:id.toString("base64"),alias})],{cwd,env});
  execFileSync(app,["app.Robot","Add",JSON.stringify({alias:"arm-01",tenant:{id:acme.toString("base64")}})],{cwd,env});
  await start({});
  assert.equal((await fetch(url(main,"/teapot"))).status,418);await stop();
  await start({web:{listen:"127.0.0.1:"+browser,origins:[origin]}});
  for(const p of [main,browser]){
    const mine=await call(p,route,"acme");assert.equal(mine.status,200);assert.deepEqual((await mine.json()).items.map(v=>v.alias),["arm-01"]);
    const other=await call(p,route,"elsewhere");assert.equal(other.status,200);assert.equal((await other.json()).items?.length??0,0);
  }
  const missing=await call(browser,"/app.RobotService/Get","acme",{ref:{slug:{tenant:{alias:"acme"},alias:"not-here"}}});
  assert.equal(missing.status,404);assert.equal((await missing.json()).code,"not_found");
  for(const path of [route,"/login"]){
    const pre=await fetch(url(browser,path),{method:"OPTIONS",headers:{origin,"access-control-request-method":"POST","access-control-request-headers":"authorization"}});
    assert.equal(pre.headers.get("access-control-allow-origin"),origin);
  }
  const login=await fetch(url(browser,"/login"),{headers:{origin}});assert.equal(await login.text(),"token");assert.equal(login.headers.get("access-control-allow-origin"),origin);
  const wire=await fetch(url(browser,route),{method:"POST",headers:{"content-type":"application/grpc-web+proto","authorization":"Bearer acme"},body:Buffer.alloc(5)});
  assert.equal(wire.status,200);assert.match(wire.headers.get("content-type"),/grpc-web/);
  const binary=Buffer.from(await wire.arrayBuffer());assert(binary.includes(Buffer.from("arm-01")));assert(!binary.includes(Buffer.from('"alias"')));
  await grpc(browser);
  execFileSync(app,["app.Robot","Add",JSON.stringify({alias:"arm-02",tenant:{id:elsewhere.toString("base64")}})],{cwd,env});
  assert.deepEqual((await (await call(main,route,"acme")).json()).items.map(r=>r.alias),["arm-01"]);
  await stop();
  env.FLEET_CONSOLE_TENANTS=[acme,elsewhere].map(b=>b.toString("hex").replace(/^(.{8})(.{4})(.{4})(.{4})(.{12})$/,"$1-$2-$3-$4-$5")).join(",");
  await start({});
  const expanded=await (await call(main,route,"acme")).json();assert.deepEqual(expanded.items.map(r=>r.alias).sort(),["arm-01","arm-02"]);
  const denied=await call(main,"/app.RobotService/Add","acme",{alias:"console-write",tenant:{id:acme.toString("base64")}});
  assert.equal(denied.status,403);assert.match((await denied.json()).message,/reads and does not write/);
  console.log("Primary and browser RPCs, tenant scope, gRPC-Web, HTTP/2 gRPC and application routes passed.");
}finally{await stop();await rm(cwd,{recursive:true,force:true});}
