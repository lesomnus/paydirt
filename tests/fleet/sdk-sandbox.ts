import { openSandbox } from "../rust/client.js";
import { app } from "./client.js";

// The SDK reference page expects one seeded Acme tenant and the Plain test credential.
export async function start(_url:string,onProgress?:(value:unknown)=>void) {
    const id="0199c3f42a108abc8a019f2e1c4d5b6a";
    const binary = btoa(String.fromCharCode(...(id.match(/../g) ?? []).map(b=>parseInt(b,16))));
    const box=await openSandbox({"sdk-admin":id},"sdk",{allowGeneralWrites:true,onProgress,seeds:[{entity:"app.Tenant",row:{id:binary,alias:"acme"}}]});
    return Object.assign(box,{app:app(box.transport)});
}

