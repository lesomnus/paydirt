// Exercise the app-owned badge verifier with a real signed identity from the fixture issuer.
import {execFileSync} from "node:child_process";
import assert from "node:assert/strict";
import {readFile,writeFile,unlink} from "node:fs/promises";
import {createFileRegistry,fromBinary} from "@bufbuild/protobuf";
import {FileDescriptorSetSchema} from "@bufbuild/protobuf/wkt";
import {Code,createClient} from "@connectrpc/connect";
import {createConnectTransport} from "@connectrpc/connect-web";
export async function presenceCases({address,call,jwt,tenant,person,device,revocation,url}) {
  const fresh=jwt(),subject=person.toString("hex");
  const proof=(reference,extra={})=>({"payday-presence":reference,"payday-subject":subject,...extra});
  const refused=async response=>assert.equal((await response.json()).code,"permission_denied");
  const recent=await call("Recent",fresh,proof("live"));
  assert.equal(recent.status,200,await recent.clone().text());
  await refused(await call("Recent",fresh,proof("stale")));
  await refused(await call("Recent",jwt({auth_time:Math.floor(Date.now()/1000)-400}),proof("live")));
  await refused(await call("Recent",fresh,proof("unknown")));
  await refused(await call("Recent",fresh,proof("live",{"payday-subject":tenant.toString("hex")})));
  await refused(await call("Recent",fresh,{"payday-presence":"live"}));
  await refused(await call("Pause",fresh,proof("live")));
  await refused(await call("Move",fresh,proof("live")));
  const offline=proof("offline",{"payday-recorded":"2026-01-03T12:00:00Z"});
  const robot={alias:"offline-signed",tenant:{id:tenant.toString("base64")}};
  await refused(await call("Add",fresh,offline,{...robot,alias:"changed"}));
  await refused(await call("Add",fresh,{...offline,"payday-recorded":"2027-01-03T12:00:00Z"},robot));
  const added=await call("Add",fresh,offline,robot);assert.equal(added.status,200,await added.clone().text());
  const audit=execFileSync("python3",["-c",'import sqlite3,sys; row=sqlite3.connect(sys.argv[1]).execute("select hex(f8), hex(f23), f25 from pd_app__Audit where f10=? order by f15 desc limit 1",("/app.RobotService/Add",)).fetchone();print(*row)',url],{encoding:"utf8"}).trim().toLowerCase();
  assert.equal(audit,[subject,device.toString("hex"),"offline"].join(" "));
  const schema=await readFile(process.env.CELL_PROJECT+"/generated/storage/descriptors.binpb");
  const service=createFileRegistry(fromBinary(FileDescriptorSetSchema,schema)).getService("app.RobotService");
  const client=createClient(service,createConnectTransport({baseUrl:"http://"+address}));
  for(const reference of ["short","live"]) {
    const stream=client.presenceWait({}, {headers:{authorization:"Bearer "+fresh,...proof(reference)},timeoutMs:4000})[Symbol.asyncIterator]();
    assert.equal((await stream.next()).value.value,1);
    if(reference==="live")await writeFile(revocation,"revoked");
    await assert.rejects(stream.next(),e=>e.code===Code.PermissionDenied);
  }
  await refused(await call("Recent",fresh,proof("live")));
  await unlink(revocation);
}
