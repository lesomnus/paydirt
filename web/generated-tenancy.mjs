import assert from "node:assert/strict";
import { createClient, Code } from "@connectrpc/connect";

export async function exerciseTenancy(services,transport,primary,other,allowedSet) {
  const [robots,cells,pairings,readings]=["Robot","Cell","Pairing","Reading"].map(n=>createClient(services[n+"Service"],transport));
  const ref=id=>({key:{case:"id",value:id}});
  const opts=token=>({headers:{authorization:"Bearer "+token}});
  const own=opts(primary.token), foreign=opts(other.token), operator=opts("both"), grouped=opts("grouped");
  const cell=await cells.add({id:allowedSet,tenant:ref(primary.id),alias:"allowed"},own);
  const hidden=await cells.add({tenant:ref(primary.id),alias:"outside-group"},own);
  const add=(tenant,cell,alias,ctx)=>robots.add({tenant:ref(tenant),cell:cell && ref(cell),alias},ctx);
  const left=await add(primary.id,cell.id,"arm-a",own);
  const right=await add(primary.id,hidden.id,"arm-b",own);
  const beta=await add(other.id,null,"arm-c",foreign);
  await robots.get({ref:ref(left.id)},grouped);
  await assert.rejects(robots.get({ref:ref(right.id)},grouped),e=>e.code===Code.NotFound);
  await assert.rejects(robots.move({ref:ref(left.id),to:ref(hidden.id)},grouped),e=>e.code===Code.NotFound);
  await assert.rejects(cells.get({ref:ref(hidden.id)},grouped),e=>e.code===Code.NotFound);
  const pair=(follow,ctx)=>pairings.add({lead:ref(left.id),follow:ref(follow)},ctx);
  await pair(right.id,operator);
  await assert.rejects(pair(beta.id,own),e=>e.code===Code.NotFound);
  await assert.rejects(pair(beta.id,operator),e=>e.code===Code.InvalidArgument);
  const row=await readings.add({robot:ref(left.id),tenantId:other.id,celsius:10},own);
  assert.deepEqual([...row.tenantId],[...primary.id]);
  await assert.rejects(readings.get({ref:ref(row.id)},foreign),e=>e.code===Code.NotFound);
  await assert.rejects(readings.add({celsius:10},own),e=>e.code===Code.InvalidArgument);
  await assert.rejects(robots.add({tenant:ref(primary.id),alias:"fake-stamp",dateAttested:{seconds:1n,nanos:0}},own),e=>e.code===Code.InvalidArgument);
}
