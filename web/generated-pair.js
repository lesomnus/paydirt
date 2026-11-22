export async function runPair(client,opts,tenant) {
  const ref=id=>({key:{case:"id",value:id}});
  const first=await client.add({tenant:ref(tenant),alias:"pair-first"},opts);
  const second=await client.add({tenant:ref(tenant),alias:"pair-second"},opts);
  const req={first:ref(first.id),second:ref(second.id),firstAlias:"pair-changed",secondAlias:"pair-changed"};
  let code=0;
  try{await client.renamePair(req,opts)}catch(e){code=e.code}
  if(code!==6)throw Error("expected unique constraint failure on second write, got "+code);
  const after=await client.get({ref:ref(first.id)},opts);
  if(after.alias!=="pair-first"||after.dateUpdated.seconds!==first.dateUpdated.seconds||after.dateUpdated.nanos!==first.dateUpdated.nanos)throw Error("failed call changed first row");
  const oldSecond=await client.get({ref:ref(second.id)},opts);
  if(oldSecond.alias!=="pair-second")throw Error("failed call changed second row");
  // The SQL harness counts only the two writes of this successful call.
  req.secondAlias="pair-other";
  const ok=await client.renamePair(req,opts);
  if(ok.alias!==req.secondAlias)throw Error("committed pair did not reach second row");
  return [first.id,second.id];
}
