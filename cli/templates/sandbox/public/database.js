import init, { AppDatabase } from "./pkg/storage.js";

self.onmessage = async ({ data, ports: [service] }) => {
  try {
    await init({module_or_path:data.module});
    const db = await AppDatabase.open(JSON.stringify(data.tenants), JSON.stringify(data.seeds ?? []));
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
