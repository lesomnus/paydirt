import assert from "node:assert/strict";
import {spawn} from "node:child_process";
import {setTimeout as delay} from "node:timers/promises";
import {mkdtemp,writeFile,rm} from "node:fs/promises";
import {join} from "node:path";
import {tmpdir} from "node:os";
import {createClient} from "@connectrpc/connect";
import {createConnectTransport} from "@connectrpc/connect-web";
import {accountCases} from "./generated-account.js";

export async function filePolicy(app,cwd,env,membership,tenants,registry){
  const dir=await mkdtemp(join(tmpdir(),"paydirt-policy-"));
  const file=join(dir,"policy.yaml"),baseUrl="http://"+env.PAYDIRT_LISTEN;
  await writeFile(file,JSON.stringify(membership.policy));
  const child=spawn(app,[],{cwd,env:{...env,PAYDIRT_POLICY_FILE:file,PAYDIRT_POLICY_REFRESH_MS:"40"},stdio:["ignore","inherit","ignore"]});
  const done=new Promise(resolve=>child.once("exit",resolve));
  try{
    let ready=false;for(let i=0;i<150&&child.exitCode===null;i++){try{await fetch(baseUrl);ready=true;break}catch{await delay(50)}}
    assert(ready);
    const client=name=>createClient(registry.getService("app."+name+"Service"),createConnectTransport({baseUrl,useBinaryFormat:true}));
    const robot=client("Robot");
    await accountCases(client("Holder"),robot,tenants,true);
    const opt={headers:{authorization:"Bearer member","paydirt-tenant":tenants[0]}};
    const {items:[row]}=await robot.list({size:1},opt);
    const stream=robot.watch({filters:[{ref:{key:{case:"id",value:row.id}}}]},{...opt,timeoutMs:8000})[Symbol.asyncIterator]();
    assert.equal((await stream.next()).value.items.length,1);
    await writeFile(file,"bindings: [");
    await delay(150);
    assert.equal((await robot.list({size:1},opt)).items.length,1);
    await writeFile(file,"roles: {}\ngroups: {}\nbindings: []\n");
    await assert.rejects(stream.next(),e=>e.code===16);
    await assert.rejects(robot.list({},opt),e=>e.code===7);
  }finally{child.kill();await done;await rm(dir,{recursive:true,force:true})}
}

