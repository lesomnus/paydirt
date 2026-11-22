import init, { AppPort } from "./pkg/app.js";

let loaded;
let endpoint;
let principals;
let name;
let generalWrites;
let policy;
let accountHeader;
const nonce = new Uint32Array(1);
do { crypto.getRandomValues(nonce); } while (!nonce[0]);

self.onmessage = async ({ data, ports: [port] }) => {
  try {
    if (data.principals) {
      loaded ??= init({module_or_path:data.module});
      await loaded;
      principals = JSON.stringify(data.principals);
      name = data.name;
      generalWrites = data.generalWrites;
      policy = JSON.stringify(data.policy);
      accountHeader = data.accountHeader;
      self.paydirtQuery = payload => new Promise((resolve, reject) => {
        const reply = new MessageChannel();
        const timeout = setTimeout(() => {
          reply.port1.close();
          reject(Error("database worker timed out"));
        }, 10000);
        reply.port1.onmessage = ({ data }) => {
          clearTimeout(timeout);
          reply.port1.close();
          if (data.error) reject(Error(data.error));
          else resolve(data.value);
        };
        port.postMessage({ payload }, [reply.port2]);
      });
      self.postMessage({ ready: true });
    } else if (data.drpc === "serve") {
      await loaded;
      endpoint = new AppPort(port, nonce[0], principals, name, generalWrites, policy, accountHeader);
    }
  } catch (err) {
    self.postMessage({ error: String(err) });
  }
};
