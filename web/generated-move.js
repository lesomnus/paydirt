// Run the same request sequence over native Connect and Payday's browser transport.
export async function exerciseMove(robots, cells, own, foreign, represented, restricted, tenant, otherTenant) {
  const ref = id => ({key:{case:"id",value:id}});
  async function rejects(fn, code) {
    try { await fn(); } catch (err) {
      if (err.code === code) return;
      throw err;
    }
    throw Error("expected Connect error " + code);
  }
  const source = await cells.add({tenant:ref(tenant), alias:"source"},own);
  const dest = await cells.add({tenant:ref(tenant), alias:"destination"},own);
  const hidden = await cells.add({tenant:ref(otherTenant), alias:"hidden"},foreign);
  const row = await robots.add({tenant:ref(tenant),cell:ref(source.id),alias:"scout"},own);
  const move = (to, options) => robots.move({ref:ref(row.id),to},options);
  await rejects(() => move(ref(hidden.id),own),5);
  await rejects(() => move(ref(dest.id),foreign),5);
  await rejects(() => move(ref(dest.id),represented),7);
  await rejects(() => move(ref(dest.id),restricted),7);
  const moved = await move(ref(dest.id),own);
  if (moved.cell?.id.join() !== dest.id.join()) throw Error("move failed");
  await rejects(() => move(ref(dest.id),own),3);
  const cleared = await move(undefined,own);
  if (cleared.cell) throw Error("cell was not cleared");
  await move(ref(dest.id),own);
  return {robot:Array.from(row.id), destination:Array.from(dest.id)};
}
