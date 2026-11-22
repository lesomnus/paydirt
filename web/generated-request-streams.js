import {Code,createClient} from "@connectrpc/connect";
import {open} from "@lesomnus/grpc-dgram/wasm";
import {createDrpcTransport} from "@lesomnus/grpc-dgram/transport/connect";

async function exercise(client, options){
  async function* count(n){for(let i=1;i<=n;i++)yield {value:i};}
  if((await client.collect(count(64),options)).value!==2080)throw Error("request stream did not collect all 64 messages");
  if((await client.collect(count(0),options)).value!==0)throw Error("empty half-close did not finish");
  const values=[];
  for await(const row of client.converse(count(3),options))values.push(row.value);
  if(values.join()!=="2,4,6")throw Error("incorrect half-closed bidi result "+values);
  const abort=new AbortController();
  let finish;
  const pending=new Promise(resolve=>{finish=resolve});
  async function* held(){yield {value:12};await pending;}
  const stream=client.converse(held(),{...options,signal:abort.signal})[Symbol.asyncIterator]();
  try {
    if((await stream.next()).value.value!==24)throw Error("bidi waited for half-close");
    if((await client.inFlight({},options)).value!==1)throw Error("server did not keep the live bidi stream");
    abort.abort();
    let code;
    try{await stream.next()}catch(err){code=err.code}
    if(code!==Code.Canceled)throw Error("bidi cancellation status "+code);
    for(let i=0;i<100;i++){
      if((await client.inFlight({},options)).value===0)return;
      await new Promise(resolve=>setTimeout(resolve,20));
    }
    throw Error("server kept canceled request stream");
  }finally{finish();}
}
export async function requestStreams(rust, service, opts, compareGo){
  if(compareGo){
    const sock=await open("/stream-reference.wasm",{worker:true,wasmExec:"/wasm_exec.js"});
    try{const conn=sock.dial();try{await exercise(createClient(service,createDrpcTransport(conn)),{});}finally{conn.close();}}finally{sock.close();}
  }
  await exercise(rust,opts);
}
