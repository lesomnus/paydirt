import assert from "node:assert/strict";
import {execFileSync, spawn} from "node:child_process";
import {mkdtemp, rm, writeFile} from "node:fs/promises";
import {createServer} from "node:net";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {setTimeout as delay} from "node:timers/promises";
import {randomBytes} from "node:crypto";

const directory = await mkdtemp(join(tmpdir(), "paydirt-slow-"));
const binary = process.env.GENERATED_APP;
function id() {
  const value = randomBytes(16);
  value[6] = 128; value[8] = 128; value[9] = 1;
  return value;
}
async function pass(threshold) {
  const socket = createServer();
  await new Promise(resolve => socket.listen(0,"127.0.0.1",resolve));
  const address = "127.0.0.1:"+socket.address().port;
  await new Promise(resolve => socket.close(resolve));
  const tenant = id();
  const env = {...process.env, DATABASE_URL:join(directory,"db-"+threshold), PAYDIRT_LISTEN:address,
    PAYDIRT_SLOW_MS:String(threshold), PAYDIRT_PRINCIPALS:JSON.stringify({member:tenant.toString("hex")})};
  await writeFile(join(directory,"app.json"),JSON.stringify({listen:address,service_name:"slow",watch:{broker:"memory"},allow_general_writes:true}));
  execFileSync(binary,["app.Tenant","Add",JSON.stringify({id:tenant.toString("base64"),alias:"own"})],{cwd:directory,env,stdio:"pipe"});
  const child=spawn(binary,[],{cwd:directory,env,stdio:["ignore","ignore","pipe"]});
  const exited=new Promise(resolve=>child.once("close",resolve));
  let diagnostic="";
  child.stderr.setEncoding("utf8").on("data",chunk=>diagnostic+=chunk);
  try {
    let ready=false;
    for(let i=0;i<150 && child.exitCode===null;i++){
      try{await fetch("http://"+address);ready=true;break;}catch{await delay(30)}
    }
    assert(ready,"server did not start");
    const add=async tenant=>fetch("http://"+address+"/app.RobotService/Add",{method:"POST",
      headers:{"content-type":"application/json",authorization:"Bearer member","connect-protocol-version":"1"},
      body:JSON.stringify({tenant:{id:tenant.toString("base64")},alias:"sensitive-business-label"})});
    const refusal=await add(id());
    assert.equal((await refusal.json()).code,"not_found");
    const allowed=await add(tenant);
    const row=await allowed.json();
    assert.equal(allowed.status,200,JSON.stringify(row));
    assert(row.id);
  } finally {child.kill();await exited;}
  const lines=diagnostic.split("\n").filter(v=>v.startsWith("{")).map(v=>JSON.parse(v)).filter(v=>v.msg==="slow");
  if(threshold===0){
    const methods=lines.map(v=>v["rpc.method"]);
    assert(methods.includes("/app.RobotService/Add"),diagnostic);
    assert(methods.includes("/app.TenantService/Get"),diagnostic);
    assert(lines.every(v=>v.level==="WARN" && Number.isInteger(v.elapsed) && v.elapsed>=0));
    assert(!diagnostic.includes("sensitive-business-label"));
  } else assert.deepEqual(lines,[]);
}
try {await pass(0);await pass(3_600_000);}
finally {await rm(directory,{recursive:true,force:true});}
