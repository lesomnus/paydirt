import init, { Database } from "./pkg/paydirt_storage.js";

self.onmessage = async ({ ports: [service] }) => {
  try {
    await init();
    const db = await Database.open();
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
