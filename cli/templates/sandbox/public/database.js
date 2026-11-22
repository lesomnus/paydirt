import init, { AppDatabase } from "./pkg/storage.js";

self.onmessage = async ({ data, ports: [service] }) => {
  try {
    await init();
    const db = await AppDatabase.open(JSON.stringify(data.tenants));
    service.onmessage = ({ data: { payload }, ports: [reply] }) => {
      try {
        reply.postMessage({ value: db.execute(payload) });
      } catch (err) {
        reply.postMessage({ error: String(err) });
      } finally {
        reply.close();
      }
    };
    self.postMessage({ ready: true });
  } catch (err) {
    self.postMessage({ error: String(err) });
  }
};
