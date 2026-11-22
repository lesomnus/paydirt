import assert from "node:assert/strict";
import {execFile,execFileSync,spawn} from "node:child_process";
import {createServer} from "node:http";
import {promisify} from "node:util";
import {readFile} from "node:fs/promises";
import {createFileRegistry,fromBinary,getOption} from "@bufbuild/protobuf";
import {FileDescriptorSetSchema} from "@bufbuild/protobuf/wkt";
import {createInterface} from "node:readline";

export async function operatorScenario(app,workspace,env,primary){
  const run=(args,extra={})=>execFileSync(app,args,{cwd:workspace,env:{...env,...extra},encoding:"utf8",stdio:["pipe","pipe","pipe"]}).trim();
  assert.match(run(["robot","--help"],{PAYDIRT_URL:"invalid",DATABASE_URL:"invalid://"}),/watch/);
  assert(!run(["cell","--help"]).includes("watch"));
  const tenant=primary.id.toString("hex");
  const idText=tenant.replace(/^(.{8})(.{4})(.{4})(.{4})(.{12})$/,"$1-$2-$3-$4-$5");
  const named="@"+tenant+"/reference-holder";
  const holder=JSON.parse(run(["holder","add",JSON.stringify({tenant:"@"+tenant,alias:"reference-holder"}),"-o","json"]));
  for(const ref of [named+"#holder",named.toUpperCase(),"slug="+JSON.stringify({tenant:"@"+tenant,alias:"reference-holder"})]){
    assert.equal(JSON.parse(run(["holder","get",ref,"-o","json"])).id,holder.id);
  }
  assert.equal(JSON.parse(run(["tenant","get","alias="+tenant,"-o","json"])).id,idText);
  assert.equal(JSON.parse(run(["tenant","get","@"+tenant+"#tenant","-o","json"])).id,idText);
  for(const args of [
    ["holder","get",named+"#robot"],["holder","get",idText],["holder","get",named+"#hodler"],
    ["holder","add",named+"#robot"],["holder","patch",named+"#robot",'{"name":"Ada"}'],
    ["holder","erase",named+"#robot"],["robot","watch",named+"#holder"],
    ["robot","move",named+"#holder",'{"to":{"alias":"floor"}}'],
  ]){
    assert.throws(()=>run(args),err=>{
      const text=String(err.stderr).toLowerCase();
      return text.includes("holder") && text.includes(args[2]===idText?"tenant":"robot");
    },args.join(" "));
  }
  assert.throws(()=>run(["holder","add","alias=surprise"]),err=>/existing row/.test(err.stderr));
  const audits=JSON.parse(run(["audit","list",JSON.stringify({filters:[{object_id:holder.id}]}),"-o","json"],{PAYDIRT_TOKEN:"audit-review"}));
  assert(audits.items.some(r=>r.objectId===holder.id));
  await uuidRequestCases(app,workspace,env,named,holder.id);
  const add=JSON.parse(run(["cell","add",JSON.stringify({tenant:{id:idText},alias:"cli"}),"-o","json"]));
  assert.equal(JSON.parse(run(["cell","get",add.id,"-o","json"])).alias,"cli");
  assert.match(run(["inventory","--help"]),/cell/);
  assert(!run(["inventory","--help"]).includes("view"));
  assert.equal(run(["inventory","view",add.id,"-o","label"]),"label=cli");
  assert.equal(run(["inventory","cell","get",add.id]),"replaced");
  assert.equal(run(["cell","get",add.id,"-o","table"]),"NAME\tRESOURCE\ncli\t"+add.id);
  const thing=JSON.parse(run(["catalog","thing","add",'{"alias":"composite-tree"}',"-o","json"]));
  assert.equal(JSON.parse(run(["shared.thing","get",thing.id,"-o","json"])).id,thing.id);
  assert.equal(run(["spotlight"]),"1");
  const zoneRow=JSON.parse(run(["robot","add",JSON.stringify({alias:"cli-time",tenant:idText}),"-o","json"]));
  assert(zoneRow.dateCreated.endsWith("Z"));
  const local=new Date(Date.parse(zoneRow.dateCreated)+9*3600000).toISOString().slice(0,19);
  const seoul=JSON.parse(run(["robot","get",zoneRow.id],{APP_TZ:"Asia/Seoul"})).dateCreated;
  assert(seoul.startsWith(local) && seoul.endsWith("+09:00"));
  assert.equal(Date.parse(JSON.parse(run(["robot","get",zoneRow.id,"--tz","UTC"],{APP_TZ:"Asia/Seoul"})).dateCreated),Date.parse(zoneRow.dateCreated));
  assert.throws(()=>run(["robot","add","--tz","Missing/Zone","{}"]),e=>/--tz/.test(e.stderr));
  assert.equal(run(["cell","get",add.id,"-o","template={{.id}} {{.alias}}"]),add.id+" cli");
  assert.match(run(["cell","get",add.id,"-o","prototext"]),/alias:\s*"cli"/);
  assert.equal(JSON.parse(run(["cell","patch",add.id,'alias: "text-edit"',"--in","prototext","-o","json"])).alias,"text-edit");
  assert.match(run(["robot","--help"]),/move/);
  const batch=JSON.parse(run(["robot","addmany",JSON.stringify({items:[{tenant:{id:idText},alias:"cli-batch"}]}),"-o","json"]));
  assert.equal(batch.items[0].alias,"cli-batch");
  assert.equal(JSON.parse(run(["cell","patch",add.id,'{"alias":"cli-edit"}',"-o","json"])).alias,"cli-edit");
  assert.equal(JSON.parse(run(["cell","erase",add.id,"-o","protojson"])).erased,true);
  assert.throws(()=>run(["cell","get",add.id]),e=>e.status!==0 && /not.?found/i.test(e.stderr));
  const stdout=JSON.parse(run(["robot","ls","--size","1","--order","alias","-o","protojson"]));
  assert.equal(stdout.items.length,1);
  const watcher=spawn(app,["robot","watch","@"+tenant+"/scout","-o","protojson"],{cwd:workspace,env,stdio:["ignore","pipe","pipe"]});
  const done=new Promise(resolve=>watcher.once("exit",resolve));
  const lines=createInterface({input:watcher.stdout});
  let timer;
  try{
    const message=await Promise.race([new Promise(resolve=>lines.once("line",resolve)),done.then(code=>{throw Error("watch exited "+code)}),new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error("watch snapshot timed out")),10000);})]);
    assert.equal(JSON.parse(message).items[0].value.alias,"scout");
  } finally {clearTimeout(timer);lines.close();watcher.kill();await done;}
}

async function uuidRequestCases(app,workspace,env,ref,uuid){
  const schema=JSON.parse(await readFile(workspace+"/generated/storage/admission.json","utf8"));
  const registry=createFileRegistry(fromBinary(FileDescriptorSetSchema,await readFile(workspace+"/generated/storage/descriptors.binpb")));
  const uuidType=registry.getEnum("orm.Type").values.find(v=>v.name==="TYPE_UUID").number;
  const fieldOptions=(type,field)=>getOption(registry.getMessage(type).field[field],registry.getExtension("orm.field"));
  for(const type of ["app.HolderAddRequest","app.HolderPatchRequest"]){
    const options=fieldOptions(type,"account");
    assert.equal(options.type,uuidType);
    assert(!options.key && !options.nullable);
  }
  assert.equal(fieldOptions("app.AuditFilter","objectId").type,uuidType);
  const captured=[];
  const receiver=createServer(async(req,res)=>{
    const chunks=[];
    for await(const chunk of req)chunks.push(chunk);
    const method=req.url.split("/").at(-1);
    const name="app.Holder"+method+"Request";
    const message=registry.getMessage(name);
    assert(message);
    assert.equal(schema[name].fields.find(f=>f.name==="account").uuid,true);
    captured.push(fromBinary(message,Buffer.concat(chunks)));
    res.writeHead(403,{"Content-Type":"application/json"});
    res.end(JSON.stringify({code:"permission_denied",message:"account assignment belongs to the deployment"}));
  });
  await new Promise(resolve=>receiver.listen(0,"127.0.0.1",resolve));
  const wire=Buffer.from(uuid.replaceAll("-",""),"hex"),base64=wire.toString("base64");
  try{
    for(const [verb,value,strict] of [["add",uuid,false],["patch",uuid,false],["add",base64,false],["add",base64,true]]){
      const args=["holder",verb,...(strict?["--in","protojson"]:[]),ref,JSON.stringify({account:value})];
      await assert.rejects(promisify(execFile)(app,args,{cwd:workspace,env:{...env,PAYDIRT_URL:"http://127.0.0.1:"+receiver.address().port}}),e=>/account assignment/.test(e.stderr));
      assert.deepEqual(Buffer.from(captured.at(-1).account),wire);
    }
    assert.equal(captured.length,4);
    assert.equal(schema["app.AuditFilter"].fields.find(f=>f.name==="objectId").uuid,true);
  }finally{await new Promise(resolve=>receiver.close(resolve));}
}
