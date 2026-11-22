import {Store} from "/upstream/ts/src/store/index.ts";
import {Queries} from "/upstream/ts/src/query/query.ts";
import {createConnectTransport} from "@connectrpc/connect-web";
import {createFileRegistry,fromBinary} from "@bufbuild/protobuf";
import {FileDescriptorSetSchema} from "@bufbuild/protobuf/wkt";
export async function who(identity,bytes){
  const service=createFileRegistry(fromBinary(FileDescriptorSetSchema,new Uint8Array(bytes))).getService("app.RobotService");
  const store=Store.open([],{name:"fleet",identity});
  const q=new Queries(store,createConnectTransport({baseUrl:location.origin,useBinaryFormat:true}),[],{services:[service]});
  try{return (await q.call(service.method.who,{})).value}finally{q.close()}
}
