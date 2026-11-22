// Run against the app-owned layer built into the external client and server.
export async function operationCases(robot,events,facts,access,own,foreign,tenant,legacy){
    const ref=id=>({key:{case:"id",value:id}});
    const same=(a,b)=>a?.length===b?.length && a.every((v,i)=>v===b[i]);
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
    const raw=await get(id);
    if(raw.desc)throw Error("a generated Get persisted the display sentence");
    for(const [client,methods] of [[events,["add","addMany"]],[facts,["add","addMany","apply","patch"]]]) {
        for(const method of methods){
            try {await client[method]({},own);throw Error("hand-written past accepted");}
            catch(e){if(e.code!==7 || !e.message.includes("a past is written by the past package"))throw e;}
        }
    }
    const review={headers:{authorization:"Bearer audit-review"}};
    const friend=await robot.add({tenant:ref(tenant),alias:"second-name"},own);
    const pair=await robot.describe({id,subject:friend.id},own);
    if(!pair.sentence.includes("renamed-operation + second-name"))throw Error("sentence missed a named subject");
    const seen=async()=>{
        const rows=[];let after="";
        do {const page=await access.list({after,size:100},review);rows.push(...page.items);after=page.next;}while(after?.length);
        return rows;
    };
    const until=Date.now()+5000;
    let reason,named;
    do {
        const rows=await seen();
        reason=rows.find(r=>r.action==="/app.EventService/Get" && r.kind==="read" && r.subjects.some(s=>same(s,id)));
        named=rows.find(r=>r.action==="/app.RobotService/Describe" && r.kind==="read" && r.subjects.length===2 && r.subjects.some(s=>same(s,subject.id)) && r.subjects.some(s=>same(s,friend.id)));
        if(reason&&named)break;
        await new Promise(resolve=>setTimeout(resolve,75));
    }while(Date.now()<until);
    if(!reason)throw Error("reading the reason omitted the event from Access");
    if(!named)throw Error("the sentence omitted its two named subjects from Access");
    if(!same(named.tenantId,tenant))throw Error("the sentence was filed under the wrong tenant");
    if(legacy){
        const previous=op(), empty=op();
        legacy.load(id,previous,empty);
        for(const options of [own,ko])if((await robot.describe({id:previous},options)).sentence!=="alice oiled arm-01")throw Error("legacy sentence changed with locale");
        legacy.rename("Alicia");
        if((await robot.describe({id:previous},own)).sentence!=="Alicia oiled arm-01")throw Error("legacy sentence kept an obsolete name");
        legacy.rename("alice");
        if((await robot.describe({id:empty},own)).sentence!=="robot.oil")throw Error("legacy event without a sentence lost its kind");
    }
    await robot.journal({id,redact:true},own);
    if((await get(id)).reason)throw Error("reason not redacted");
}
