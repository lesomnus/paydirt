import { Code, ConnectError, createClient } from "@connectrpc/connect";
import { createConnectTransport } from "@connectrpc/connect-web";
import { RobotService } from "./upstream/internal/apptest/ts/gen/app/robot_svc_pb.ts";
import { openSandbox } from "./sandbox.js";

function assert(ok, message) {
  if (!ok) throw Error(message);
}

async function rejects(operation, code) {
  try {
    await operation();
  } catch (err) {
    if (ConnectError.from(err).code !== code) throw err;
    return;
  }
  throw Error("expected Connect code " + code);
}

const byId = id => ({ key: { case: "id", value: id } });
const bySlug = (tenant, alias) => ({ key: { case: "slug", value: { tenant, alias } } });
const bytes = hex => Uint8Array.from(hex.match(/../g), pair => parseInt(pair, 16));
const options = token => ({ headers: { authorization: "Bearer " + token } });

window.exercise = async (mode, principals) => {
  const [[first, a], [second, b]] = Object.entries(principals);
  const owner = options(first), other = options(second);
  const tenant = byId(bytes(a)), foreign = byId(bytes(b));
  let sandbox;
  try {
    if (mode === "sqlite") sandbox = await openSandbox(principals);
    const transport = sandbox?.link.transport ?? createConnectTransport({
      baseUrl: location.origin, useBinaryFormat: true,
    });
    const client = createClient(RobotService, transport);
    const get = (id, opts = owner) => client.get({ ref: byId(id) }, opts);
    await rejects(() => client.list({}), Code.Unauthenticated);
    await rejects(() => client.add({ tenant: foreign, alias: "cross" }, owner), Code.NotFound);
    const row = await client.add({ tenant, alias: "scout" }, owner);
    assert(row.id.length === 16 && row.id[6] >> 4 === 8 && row.id[9] === 7, "generated Robot pdid");
    assert((await get(row.id)).alias === "scout", "read by id");
    const slug = name => client.get({ ref: bySlug(tenant, name) }, owner);
    assert((await slug("scout")).id.join() === row.id.join(), "read by tenant slug");
    await rejects(() => get(row.id, other), Code.NotFound);
    await rejects(() => client.add({ tenant, alias: "extra", keeper: "unsupported" }, owner), Code.Unimplemented);
    const before = await client.list({}, owner);
    assert(before.items.length === 1 && before.next === "", "first page");

    const stop = new AbortController();
    const firstMessage = filters => client.watch({ filters }, owner)[Symbol.asyncIterator]().next();
    await rejects(() => firstMessage([{ tenant }]), Code.InvalidArgument);
    await rejects(() => firstMessage([{ ref: bySlug(tenant, "absent") }]), Code.NotFound);
    const watch = client.watch({ filters: [{ ref: bySlug(tenant, "scout") }] }, { ...owner, signal: stop.signal })[Symbol.asyncIterator]();
    const snapshot = await watch.next();
    assert(snapshot.value.items.length === 1 && snapshot.value.items[0].value.alias === "scout"
      && snapshot.value.items[0].action === "", "initial snapshot");

    await client.add({ tenant: foreign, alias: "scout" }, other);
    await rejects(() => client.addMany({ items: [
      { tenant, alias: "rolled-back" }, { tenant, alias: "scout" },
    ] }, owner), Code.AlreadyExists);
    await rejects(() => slug("rolled-back"), Code.NotFound);
    const batch = await client.addMany({ items: [{ tenant, alias: "loader" }, { tenant, alias: "welder" }] }, owner);
    assert(batch.items.map(r => r.alias).join() === "loader,welder", "atomic batch result order");
    await rejects(() => client.patch({ ref: byId(row.id), alias: "loader" }, owner), Code.AlreadyExists);
    await client.patch({ ref: byId(row.id), alias: "ranger" }, owner);
    const changed = (await watch.next()).value.items;
    assert(changed.length === 1 && changed[0].value.alias === "ranger"
      && changed[0].id.join() === row.id.join() && changed[0].action === "/app.RobotService/Patch",
      "slug watch followed the original ID and reported the committed patch");
    assert(!(await client.erase(byId(row.id), other)).erased, "cross-tenant erase");
    assert((await client.erase(byId(row.id), owner)).erased, "first erase");
    const removed = (await watch.next()).value.items;
    assert(removed.length === 1 && !removed[0].value && removed[0].id.join() === row.id.join()
      && removed[0].action === "/app.RobotService/Erase", "committed removal");
    assert(!(await client.erase(byId(row.id), owner)).erased, "idempotent erase");
    await rejects(() => get(row.id), Code.NotFound);
    const replacement = await client.add({ tenant, alias: "ranger" }, owner);
    assert(replacement.id.join() !== row.id.join(), "erasure released the alias");
    stop.abort();
    await rejects(() => watch.next(), Code.Canceled);

    const pages = [];
    let after = "";
    do {
      const result = await client.list({ size: 1, after }, owner);
      assert(result.items.length === 1, "single row page");
      pages.push(result.items[0].id.join());
      after = result.next;
    } while (after && pages.length <= 3);
    assert(!after && pages.length === 3 && new Set(pages).size === 3, "keyset pages");
    if (sandbox) {
      sandbox.link.reachable = false;
      await rejects(() => get(replacement.id), Code.Unavailable);
      sandbox.link.reachable = true;
      assert((await get(replacement.id)).alias === "ranger", "SandboxLink reconnect");
    }
    return Array.from(replacement.id);
  } finally {
    sandbox?.close();
  }
};

window.afterReload = async (principals, id) => {
  const session = await openSandbox(principals);
  try {
    const client = createClient(RobotService, session.link.transport);
    const row = await client.get({ ref: byId(Uint8Array.from(id)) }, options(Object.keys(principals)[0]));
    assert(row.alias === "ranger", "row persisted across page and worker restart");
  } finally {
    session.close();
  }
};
