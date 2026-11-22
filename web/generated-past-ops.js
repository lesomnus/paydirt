// Run against the app-owned layer built into the external client and server.
export async function operationCases(robot,events,own,foreign,tenant){
    const ref=id=>({key:{case:"id",value:id}});
    const same=(a,b)=>String(a)===String(b);
    const op=()=>{const bytes=crypto.getRandomValues(new Uint8Array(16));bytes[6]=128;bytes[8]=128;bytes[9]=17;return bytes;};
    const subject=await robot.add({tenant:ref(tenant),alias:"operation-robot"},own);
    const id=op();
    const call={id,subject:subject.id};
    try{await robot.journal({...call,fail:true},own);throw Error("failed transaction committed")}catch(e){if(e.code!==10)throw e}
    const both=await Promise.all([robot.journal(call,own),robot.journal(call,own)]);
    if(both.filter(r=>r.done).length!==1)throw Error("duplicate operation");
    const root=both.find(r=>!r.done);
    const get=(id,opts=own)=>events.get({ref:ref(id)},opts);
    if(!same((await get(root.child)).causeId,id)||(await get(root.detached)).causeId?.length)throw Error("incorrect nested or detached cause");
    if((await get(id)).reason!=="manual review")throw Error("event did not record its reason");
    try{await robot.journal({id:op(),subject:subject.id},foreign);throw Error("foreign subject visible")}catch(e){if(e.code!==5)throw e}
    const ko={...own,headers:{...own.headers,"accept-language":"ko-KR,en;q=0.8"}};
    if((await robot.describe({id},ko)).sentence!=="ko operation-robot 09:00")throw Error("event language or timezone");
    await robot.patch({ref:ref(subject.id),dateUpdated:subject.dateUpdated,alias:"renamed-operation"},own);
    if((await robot.describe({id},own)).sentence!=="en renamed-operation 09:00")throw Error("stale event display name");
    await robot.journal({id,redact:true},own);
    if((await get(id)).reason)throw Error("reason not redacted");
}
