import { dialWorker } from "@lesomnus/grpc-dgram/transport/port";
import { createDrpcTransport } from "@lesomnus/grpc-dgram/transport/connect";
import { SandboxLink } from "./link.ts";

function start(worker, data, port) {
  return new Promise((resolve, reject) => {
    worker.onerror = e => reject(Error(e.message));
    worker.onmessage = ({ data }) => data.ready ? resolve() : reject(Error(data.error));
    worker.postMessage(data, [port]);
  });
}

export async function openSandbox(principals, name = "paydirt") {
  const database = new Worker("/database.js", { type: "module", name: "paydirt-sqlite" });
  const server = new Worker("/server.js", { type: "module", name: "paydirt-server" });
  const channel = new MessageChannel();
  try {
    await Promise.all([
      start(database, { tenants: [...new Set(Object.values(principals))] }, channel.port1),
      start(server, { principals, name }, channel.port2),
    ]);
    const conn = dialWorker(server);
    const link = new SandboxLink(createDrpcTransport(conn));
    return { link, close() {
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
