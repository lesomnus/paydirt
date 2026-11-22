// A content hash in the build manifest keeps cached modules tied to their generated bindings.
export async function modules(onProgress) {
  const manifest = await fetch("/pkg/modules.json", {cache:"no-cache"});
  if (!manifest.ok) throw Error("Build the sandbox WASM modules first");
  const entries = Object.entries(await manifest.json());
  const total = entries.reduce((size,[,module])=>size+module.bytes,0);
  const cache = await globalThis.caches?.open("paydirt-wasm-v1").catch(()=>undefined);
  let loaded = 0;
  const result = {};
  for (const [name,entry] of entries) {
    const url = new URL("/pkg/"+name+"_bg.wasm",location.href);
    url.searchParams.set("sha256",entry.sha256);
    let response=await cache?.match(url);
    const from=response?"cache":"network";
    response??=await fetch(url);
    if (!response.ok) throw Error("could not load "+name);
    const chunks=[], reader=response.body.getReader();
    let size=0;
    for (;;) {
      const {value,done}=await reader.read();
      if(done)break;
      chunks.push(value);size+=value.byteLength;loaded+=value.byteLength;
      onProgress?.({loaded,total,from,keeping:!!cache});
    }
    const bytes=new Uint8Array(size);
    let at=0;for(const chunk of chunks){bytes.set(chunk,at);at+=chunk.byteLength;}
    const hash=Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256",bytes)),n=>n.toString(16).padStart(2,"0")).join("");
    if(size!==entry.bytes || hash!==entry.sha256){await cache?.delete(url);throw Error("stale "+name+" module; reload the sandbox");}
    result[name]=await WebAssembly.compile(bytes);
    if(cache && from==="network") await cache.put(url,new Response(bytes,{headers:{"Content-Type":"application/wasm"}})).catch(()=>{});
  }
  return result;
}

