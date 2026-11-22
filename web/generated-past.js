// Only compare business values; each server mints its own subject, event and cell IDs.
export async function pastScenario({robot,cell,event,fact},tenant,opts){
  const ref=id=>({key:{case:"id",value:id}});
  const t=ref(tenant);
  const first=await cell.add({tenant:t,alias:"past-origin"},opts);
  const last=await cell.add({tenant:t,alias:"past-dest"},opts);
  const subject=await robot.add({tenant:t,cell:ref(first.id),alias:"past-robot"},opts);
  const move=to=>robot.move({ref:ref(subject.id),to},opts);
  await move(ref(last.id));
  try{await move(ref(last.id));throw Error("duplicate move accepted")}catch(e){if(e.code!==3)throw e}
  await move(undefined);
  const filters=[{subjectId:subject.id}];
  const events=(await event.list({filters},opts)).items.filter(e=>e.kind==="robot.move");
  const facts=(await fact.list({filters},opts)).items.filter(f=>f.fieldNumber===3);
  const destination=[...last.id].map(b=>b.toString(16).padStart(2,"0")).join("").replace(/^(........)(....)(....)(....)(............)$/,"$1-$2-$3-$4-$5");
  const report={events:events.map(e=>[e.kind,e.value?.typeUrl]),facts:facts.map(f=>[f.value===JSON.stringify(destination)?"destination":f.value,events.some(e=>String(e.id)===String(f.eventId))]).sort()};
  if(events.length!==2 || !report.facts.some(([v])=>v==="destination") || !report.facts.some(([v])=>v==="null")) throw Error("missing atomic move history "+JSON.stringify(report));
  return report;
}

