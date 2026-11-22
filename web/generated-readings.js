export async function checkReadings(client) {
  const first = await client.list({size: 1});
  const last = await client.list({size: 1, after: first.next});
  if (!first.next || last.next || first.items[0]?.value !== "reading-1" || last.items[0]?.value !== "reading-2") {
    throw Error("computed List lost its page boundary");
  }
  const value = await client.get({ref: {id: first.items[0].id}});
  if (value.value !== "reading-1") throw Error("computed Get returned the wrong row");
  const revisions = [];
  for await (const snapshot of client.watch({})) revisions.push([snapshot.metadata?.revision, snapshot.items.length]);
  if (JSON.stringify(revisions) !== JSON.stringify([["v1",2],["v2",0]])) throw Error("computed Watch lost its empty snapshot");
  for await (const snapshot of client.watch({revision: "v1"})) {
    if (snapshot.metadata?.revision !== "v2") throw Error("provider ignored revision");
  }
}
