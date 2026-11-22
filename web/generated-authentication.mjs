// Local discovery, keys and Connect introspection use the same external fleet server as applications.
import {presenceCases} from "./generated-presence.mjs";
import assert from "node:assert/strict";
import {createServer} from "node:http";
import {generateKeyPairSync,randomBytes,sign} from "node:crypto";
import {spawn,execFileSync} from "node:child_process";
import {mkdtemp,readFile,writeFile,rm} from "node:fs/promises";
import {chromium} from "playwright";
import {createServer as viteServer} from "vite";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {setTimeout as delay} from "node:timers/promises";
const tmp=await mkdtemp(join(tmpdir(),"fleet-auth-")),tenant=randomBytes(16);tenant[6]=128;tenant[8]=128;tenant[9]=1;
const device=Buffer.from(tenant), person=Buffer.from(tenant);device[9]=person[9]=2;person[15]^=1;
const revocation=join(tmp,"badge-revoked");
let kid="one",pair=generateKeyPairSync("rsa",{modulusLength:2048}),down=false,child;
const listen=s=>new Promise(resolve=>s.listen(0,"127.0.0.1",resolve));
const stop=s=>new Promise(resolve=>s.close(resolve));
const provider=createServer(async(req,res)=>{
 const send=(status,data)=>{res.writeHead(status,{"Content-Type":"application/json"});res.end(JSON.stringify(data))};
 if(req.url==="/.well-known/openid-configuration")return send(200,{issuer,jwks_uri:issuer+"/keys"});
 if(req.url==="/keys")return send(200,{keys:[{...pair.publicKey.export({format:"jwk"}),kid,alg:"RS256",use:"sig"}]});
 if(req.url==="/payday.TokenService/Introspect"){
   if(down||req.headers.authorization!=="Bearer service-secret")return send(503,{code:"unavailable"});
   let body="";for await(const part of req)body+=part;
   const token=JSON.parse(body).token;if(!["opaque","narrow","expired"].includes(token))return send(404,{code:"not_found"});
   return send(200,{tenant:"fleet",alias:"read",grant:{anyTenant:true,anySet:true,anyAction:token!=="narrow",actions:[]},
       dateExpires:new Date(Date.now()+(token==="expired"?-3600000:3600000)).toISOString()});
 }
 send(404,{})
});
await listen(provider);const issuer="http://127.0.0.1:"+provider.address().port;
const where=createServer();await listen(where);const address="127.0.0.1:"+where.address().port;await stop(where);
const enc=v=>Buffer.from(JSON.stringify(v)).toString("base64url");
function jwt(extra={}){const now=Math.floor(Date.now()/1000);const input=enc({alg:"RS256",kid,typ:"at+jwt"})+"."+enc({iss:issuer,sub:"user",aud:"fleet",client_id:"browser",jti:"local",iat:now,exp:now+120,auth_time:now,...extra});return input+"."+sign("sha256",Buffer.from(input),pair.privateKey).toString("base64url")}
const call=(rpc,token,extra={},body={})=>fetch("http://"+address+"/app.RobotService/"+rpc,{method:"POST",headers:{"content-type":"application/json","connect-protocol-version":"1",...(token?{authorization:"Bearer "+token}:{}),...extra},body:JSON.stringify(body)});
try{
 const config={listen:address,service_name:"auth",watch:{broker:"memory"},authentication:{oidc:{issuer,audience:"fleet"},introspection:{url:issuer,bearer_env:"FLEET_BILLING"},bindings:{"oidc:user":{tenant:tenant.toString("hex"),actor:device.toString("hex")},"remote:fleet/read":tenant.toString("hex")},sessions:true}};
 await writeFile(join(tmp,"app.json"),JSON.stringify(config));const env={...process.env,PAYDIRT_LISTEN:address,DATABASE_URL:join(tmp,"db"),FLEET_BILLING:"service-secret",FLEET_DEVICE:device.toString("hex"),FLEET_BADGE:person.toString("hex"),FLEET_REVOCATION:revocation,PAYDIRT_PRINCIPALS:"{}"};
 execFileSync(process.env.GENERATED_APP,["app.Tenant","Add",JSON.stringify({id:tenant.toString("base64"),alias:"tenant"})],{env,cwd:tmp});
 child=spawn(process.env.GENERATED_APP,[],{env,cwd:tmp,stdio:"inherit"});
 for(let i=0;i<120;i++){try{await fetch("http://"+address);break}catch{await delay(50)}}
 const ok=async(r,n)=>{assert.equal(r.status,200,await r.clone().text());assert.equal((await r.json()).value,n)};
 const denied=async(r,code)=>assert.equal((await r.json()).code,code);
 await ok(await call("Who",jwt()),2);
 await presenceCases({address,call,jwt,tenant,person,device,revocation,url:env.DATABASE_URL});
 await denied(await call("Who",jwt({aud:"wrong"})),"unauthenticated");
 await denied(await call("Who",jwt({exp:5})),"unauthenticated");
 await denied(await call("Who",jwt({sub:"unbound"})),"unauthenticated");
 await delay(1100);kid="two";pair=generateKeyPairSync("rsa",{modulusLength:2048});await ok(await call("Who",jwt()),2);
 await ok(await call("Who","opaque"),3);await denied(await call("Who","narrow"),"permission_denied");
 await denied(await call("Who","expired"),"unauthenticated");await denied(await call("Who","unknown"),"unauthenticated");
 down=true;await denied(await call("Who","opaque"),"unavailable");down=false;
 const login=await call("Login",jwt(),{},{value:15});assert.equal(login.status,200,await login.clone().text());
 const cookie=login.headers.get("set-cookie");for(const a of ["__Host-pd_session=","Secure","HttpOnly","Path=/"])assert(cookie.includes(a));
 const session={cookie:cookie.split(";")[0],"payday-identity":"oidc:user"};
 await ok(await call("Who",null,session),2);
 await denied(await call("Who",null,{...session,"payday-identity":"other"}),"unauthenticated");
 const exit=await call("Logout",null,session);assert.match(exit.headers.get("set-cookie"),/Max-Age=0/);
 await denied(await call("Who",null,session),"unauthenticated");
 const short=await call("Login",jwt(),{},{value:1});await delay(1100);
 await denied(await call("Who",null,{cookie:short.headers.get("set-cookie").split(";")[0]}),"unauthenticated");
 const sdk=await viteServer({server:{port:0,host:"127.0.0.1",proxy:{"/app.":{target:"http://"+address}}},logLevel:"error"});
 let browser;
 try{
   await sdk.listen();browser=await chromium.launch({executablePath:process.env.CHROMIUM_PATH});
   const page=await browser.newPage();
   await page.route("**/auth-check",route=>route.fulfill({body:"<!doctype html><title>Session test</title>",contentType:"text/html"}));
   await page.goto("http://127.0.0.1:"+sdk.httpServer.address().port+"/auth-check");
   const bytes=Array.from(await readFile(join(process.env.CELL_PROJECT,"generated/storage/descriptors.binpb")));
   const results=await page.evaluate(async({token,bytes})=>{
     const request=(rpc,auth)=>fetch("/app.RobotService/"+rpc,{method:"POST",headers:{"content-type":"application/json",...(auth?{authorization:"Bearer "+auth}:{})},body:'{"value":30}'});
     const login=await request("Login",token);if(!login.ok)throw Error(await login.text());
     const {who}=await import("/generated-auth-sdk.js");
     const ok=await who("oidc:user",bytes);
     async function status(identity){try{await who(identity,bytes);return 0}catch(e){return e.code}}
     const mismatch=await status("other");const visible=document.cookie;
     await request("Logout");return [ok,mismatch,visible,await status("oidc:user")];
   },{token:jwt(),bytes});
   assert.deepEqual(results,[2,16,"",16]);
 }finally{await browser?.close();await sdk.close()}
 console.log("Verified OIDC audience, expiry, rotated keys, remote grants and browser cookie sessions end to end.");
}finally{if(child){const ended=new Promise(r=>child.once("exit",r));child.kill();await ended}await stop(provider);await rm(tmp,{recursive:true,force:true})}
