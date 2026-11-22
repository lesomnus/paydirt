import assert from "node:assert/strict";
import {execFileSync,spawn} from "node:child_process";
import {createInterface} from "node:readline";

export async function operatorScenario(app,workspace,env,primary){
  const run=(args,extra={})=>execFileSync(app,args,{cwd:workspace,env:{...env,...extra},encoding:"utf8",stdio:["pipe","pipe","pipe"]}).trim();
  assert.match(run(["robot","--help"],{PAYDIRT_URL:"invalid",DATABASE_URL:"invalid://"}),/watch/);
  assert(!run(["cell","--help"]).includes("watch"));
  const tenant=primary.id.toString("hex");
  const idText=tenant.replace(/^(.{8})(.{4})(.{4})(.{4})(.{12})$/,"$1-$2-$3-$4-$5");
  const add=JSON.parse(run(["cell","add",JSON.stringify({tenant:{id:idText},alias:"cli"}),"-o","json"]));
  assert.equal(JSON.parse(run(["cell","get",add.id,"-o","json"])).alias,"cli");
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
