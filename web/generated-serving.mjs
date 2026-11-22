// Drive a generated app and a local OTLP/HTTP collector with real TLS peers.
import assert from "node:assert/strict";
import {spawn,execFileSync} from "node:child_process";
import {randomBytes} from "node:crypto";
import http from "node:http";
import https from "node:https";
import net from "node:net";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {mkdtemp,readFile,writeFile,rm} from "node:fs/promises";
import {setTimeout as delay} from "node:timers/promises";
const cwd=await mkdtemp(join(tmpdir(),"paid-listen-")), binary=process.env.GENERATED_APP;
const run=(cmd,args,env=process.env)=>execFileSync(cmd,args,{cwd,env,stdio:"pipe"});
async function port(){
 const s=net.createServer();await new Promise(r=>s.listen(0,"127.0.0.1",r));
 const p=s.address().port;await new Promise(r=>s.close(r));return p;
}
const received=[];
const collector=http.createServer(async(req,res)=>{
 const chunks=[];for await(const chunk of req)chunks.push(chunk);
 if(req.url==="/v1/traces" && req.headers["content-type"]?.includes("protobuf"))received.push(Buffer.concat(chunks));
 res.writeHead(200,{"Content-Type":"application/x-protobuf"});res.end();
});
await new Promise(r=>collector.listen(0,"127.0.0.1",r));
let child,exited;
try{
 const openssl=args=>run("openssl",args.split(" "));
 openssl("req -x509 -newkey rsa:2048 -nodes -subj /CN=test-CA -keyout ca.key -out ca.pem -days 1");
 for(const name of ["server","client"]){
   openssl(`req -newkey rsa:2048 -nodes -subj /CN=${name} -keyout ${name}.key -out ${name}.csr`);
   await writeFile(join(cwd,"ext"),name==="server"?"subjectAltName=IP:127.0.0.1\nextendedKeyUsage=serverAuth\n":"extendedKeyUsage=clientAuth\n");
   openssl(`x509 -req -in ${name}.csr -CA ca.pem -CAkey ca.key -CAcreateserial -out ${name}.pem -days 1 -extfile ext`);
 }
 const tls={cert:"server.pem",key:"server.key"}, main=await port(),web=await port();
 const config={listen:`127.0.0.1:${main}`,service_name:"fleet-observed",watch:{broker:"memory"},allow_general_writes:true,rate:{burst:2,per_second:20},
   serving:{tls:{...tls,client_ca:"ca.pem"},deadline_ms:200,web:{listen:`127.0.0.1:${web}`,tls,origins:["https://portal.test"]},otlp_traces_endpoint:`http://127.0.0.1:${collector.address().port}/v1/traces`}};
 const id=randomBytes(16);id[6]=128;id[8]=128;id[9]=1;
 const env={...process.env,DATABASE_URL:join(cwd,"db"),PAYDIRT_LISTEN:config.listen,PAYDIRT_PRINCIPALS:JSON.stringify({member:id.toString("hex")})};
 await writeFile(join(cwd,"app.json"),JSON.stringify(config));
 run(binary,["app.Tenant","Add",JSON.stringify({id:id.toString("base64"),alias:"tenant"})],env);
 const [ca,cert,key]=await Promise.all(["ca.pem","client.pem","client.key"].map(s=>readFile(join(cwd,s))));
 const traceId=randomBytes(16).toString("hex");
 function rpc(port,path,body,extras={}){
   const {client,method="POST",headers={}}=extras;
   return new Promise((resolve,reject)=>{
     const req=https.request({hostname:"127.0.0.1",port,path,method,ca,...(client?{cert,key}:{}),agent:false,
       headers:{"content-type":"application/json","connect-protocol-version":"1","authorization":"Bearer member",traceparent:"00-"+traceId+"-0123456789abcdef-01",...headers}},async res=>{
       const parts=[];for await(const c of res)parts.push(c);resolve({status:res.statusCode,headers:res.headers,body:Buffer.concat(parts).toString()});
     });req.on("error",reject);req.setTimeout(5000,()=>req.destroy(Error("request timed out")));req.end(body&&JSON.stringify(body));
   });
 }
 const settings="/app.SettingService/Get",setting={ref:{alias:"service"}};
 child=spawn(binary,[],{cwd,env,stdio:"inherit"});exited=new Promise(r=>child.once("exit",r));
 let ready=false;for(let i=0;i<160;i++){try{if((await rpc(main,settings,setting,{client:true})).status===200){ready=true;break;}}catch{}await delay(30);}
 assert(ready,"mTLS listener did not start");
 await assert.rejects(rpc(main,settings,setting));
 assert.equal(JSON.parse((await rpc(web,settings,setting)).body).value,"fleet-observed");
 for(const [origin,want] of [["https://portal.test","https://portal.test"],["https://other.test",undefined]]){
   const res=await rpc(web,settings,null,{method:"OPTIONS",headers:{origin,"access-control-request-method":"POST","access-control-request-headers":"authorization,content-type"}});
   assert.equal(res.headers["access-control-allow-origin"],want);
 }
 const item={tenant:{id:id.toString("base64")},alias:"never-added"};
 const bulk=await rpc(main,"/app.RobotService/AddMany",{items:[item,item,item]},{client:true});
 assert.equal(JSON.parse(bulk.body).code,"resource_exhausted");
 await delay(150);
 const slow=await rpc(main,"/app.RobotService/Pause",{value:900},{client:true});
 assert.equal(JSON.parse(slow.body).code,"deadline_exceeded");
 child.kill();await exited;child=undefined;
 const spans=Buffer.concat(received);
 assert(spans.includes(Buffer.from("fleet-observed")) && spans.includes(Buffer.from(settings)) && spans.includes(Buffer.from(traceId,"hex")),"collector did not receive linked RPC spans");
 config.serving.tls.key="ca.key";
 await writeFile(join(cwd,"app.json"),JSON.stringify(config));
 assert.throws(()=>run(binary,[],env));
 console.log("Native mTLS, web CORS, batch cost, deadline, invalid TLS and collected OTLP trace passed.");
}finally{
 if(child){child.kill();await exited;}
 await new Promise(r=>collector.close(r));await rm(cwd,{recursive:true,force:true});
}
