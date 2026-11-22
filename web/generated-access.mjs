import assert from "node:assert/strict";
import {Code,createClient} from "@connectrpc/connect";
import {createConnectTransport} from "@connectrpc/connect-web";
import {fromBinary} from "@bufbuild/protobuf";
export async function accessCases(registry,baseUrl,robot,cell,own,foreign,primary,other) {
    const client=n=>createClient(registry.getService(n),createConnectTransport({baseUrl,useBinaryFormat:true}));
    const ref=id=>({key:{case:"id",value:id}});
    const opts=token=>({headers:{authorization:"Bearer "+token}});
    const access=client("app.AccessService"),audit=client("app.AuditService");
    const review=opts("audit-review"),otherReview=opts("other-review");
    const list=async(c,o)=>(await c.list({size:200},o)).items;
    const denied=fn=>assert.rejects(fn,e=>e.code===Code.PermissionDenied);
    const row=await robot.add({tenant:ref(primary),alias:"audit-example",keeper:"private-keeper",secret:Buffer.from("verifier")},own);
    const alien=await robot.add({tenant:ref(other),alias:"audit-away",keeper:"other-person"},foreign);
    await robot.patch({ref:ref(row.id),dateUpdated:row.dateUpdated,alias:"audit-updated",keeper:"personal-second"},own);
    const same=(a,b)=>Buffer.from(a).equals(Buffer.from(b));
    await denied(()=>audit.list({},own)); await denied(()=>access.list({},own));
    const rows=(await list(audit,review)).filter(r=>same(r.objectId,row.id));
    assert.equal(rows.length,2);
    for (const r of rows) {
      assert(r.patch.length>0 && r.value.length>0);
      assert(r.fields.includes(32));
      const body=fromBinary(registry.getMessage("app.Robot"),r.value);assert.equal(body.keeper,"");assert.equal(body.secret.length,0);
      const patch=fromBinary(registry.getMessage("patch.Patch"),r.patch);
      assert.equal(patch.messageType,"app.Robot");
      for(const name of ["verifier","private-keeper","personal-second"])assert(!Buffer.from(r.patch).includes(name));
    }
    await assert.rejects(audit.get({ref:ref(rows[0].id)},otherReview),e=>e.code===Code.NotFound);
    const station=await cell.add({tenant:ref(primary),alias:"audit-fieldsonly"},own);
    const metadata=(await list(audit,review)).find(r=>same(r.objectId,station.id));assert(metadata.fields.length);assert.equal(metadata.patch.length+metadata.value.length,0);
    await robot.get({ref:ref(row.id)},own);
    await assert.rejects(robot.exportRobot({ref:ref(row.id)},own),e=>e.code===Code.InvalidArgument);
    const reason="case 147";
    await robot.exportRobot({ref:ref(row.id)},{headers:{...own.headers,"payday-reason-bin":Buffer.from(reason).toString("base64")}});
    await denied(()=>robot.patch({ref:ref(row.id),alias:"denied-sensitive"},opts("reader")));
    const ctrl=new AbortController();const stream=robot.watch({filters:[{ref:ref(row.id)}]},{...own,signal:ctrl.signal,timeoutMs:4000})[Symbol.asyncIterator]();
    await stream.next();ctrl.abort();await stream.return?.().catch(()=>{});
    const entries=(await list(access,review)).filter(r=>r.subjects.some(id=>same(id,row.id)));
    assert(entries.some(r=>r.kind==="write"&&r.result==="ok"));
    assert(entries.some(r=>r.action==="/app.RobotService/Watch"&&r.kind==="read"));
    const refused=entries.find(r=>r.result==="permission_denied");assert(refused);
    const detail=r=>fromBinary(registry.getMessage("payday.AccessDetail"),r.value);assert.equal(detail(refused).request.length,0);
    const download=entries.find(r=>r.kind==="download");assert(download);assert.equal(detail(download).reason,reason);assert(detail(download).request.length);
    assert(!(await list(access,review)).some(r=>r.subjects.some(id=>same(id,alien.id))));
}
