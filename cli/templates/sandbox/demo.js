import { fromBinary, createFileRegistry } from "@bufbuild/protobuf";
import { FileDescriptorSetSchema } from "@bufbuild/protobuf/wkt";
import { createClient } from "@connectrpc/connect";
import { openSandbox } from "./client.js";
const read = document.querySelector("#read");
const result = document.querySelector("#result");
let sandbox;
read.onclick = async () => {
  read.disabled = true;
  try {
    sandbox ??= await openSandbox({});
    const response = await fetch("./pkg/descriptors.binpb");
    if (!response.ok) throw Error("Run npm run build:wasm first");
    const registry = createFileRegistry(fromBinary(FileDescriptorSetSchema,new Uint8Array(await response.arrayBuffer())));
    const client = createClient(registry.getService("app.SettingService"), sandbox.link.transport);
    const setting = await client.get({ref:{key:{case:"alias",value:"service"}}});
    result.textContent = setting.value;
  } catch (err) { result.textContent = err.message; }
  finally { read.disabled = false; }
};
addEventListener("pagehide", () => sandbox?.close());
