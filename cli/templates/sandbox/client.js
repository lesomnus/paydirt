import { dialWorker } from "@lesomnus/grpc-dgram/transport/port";
import { createDrpcTransport } from "@lesomnus/grpc-dgram/transport/connect";
import { createContextKey, createContextValues } from "@connectrpc/connect";
import { modules } from "./modules.js";
import { SandboxLink } from "./link.ts";

const streamHeader = createContextKey(undefined);
export function resourceHeaders(notify) {
  return {contextValues:createContextValues().set(streamHeader,notify)};
}
function transport(conn) {
  const ordinary=createDrpcTransport(conn);
  return {...ordinary, stream(method,signal,timeout,headers,input,context) {
    const notify=context?.get(streamHeader);
    if (!notify) return ordinary.stream(method,signal,timeout,headers,input,context);
    const observed=createDrpcTransport({newStream(desc,options) {
      const stream=conn.newStream(desc,options);
      stream.header().then(md=>{if(md?.["paydirt-watch-ready"]?.includes("1"))notify();}).catch(()=>{});
      return stream;
    }});
    return observed.stream(method,signal,timeout,headers,input,context);
  }};
}
function start(worker, data, port) {
  return new Promise((resolve, reject) => {
    worker.onerror = e => reject(Error(e.message));
    worker.onmessage = ({ data }) => data.ready ? resolve() : reject(Error(data.error));
    worker.postMessage(data, [port]);
  });
}

export async function openSandbox(principals, name = "paydirt", { allowGeneralWrites = false, seeds = [], policy, accountHeader, onProgress } = {}) {
  const wasm=await modules(onProgress);
  const database = new Worker("/database.js", { type: "module", name: "paydirt-sqlite" });
  const server = new Worker("/server.js", { type: "module", name: "paydirt-server" });
  const channel = new MessageChannel();
  try {
    await Promise.all([
      start(database, { module:wasm.storage,seeds, tenants: [...new Set(Object.values(principals).map(p => typeof p === "string" ? p : p.tenant).filter(Boolean))] }, channel.port1),
      start(server, { module:wasm.app,principals, name, generalWrites: allowGeneralWrites, policy, accountHeader }, channel.port2),
    ]);
    const conn = dialWorker(server);
    const link = new SandboxLink(transport(conn));
    return { link,transport:link.transport,get reachable(){return link.reachable},set reachable(value){link.reachable=value},close() {
      link.close();
      conn.close();
      server.terminate();
      database.terminate();
    } };
  } catch (err) {
    server.terminate();
    database.terminate();
    throw err;
  }
}
