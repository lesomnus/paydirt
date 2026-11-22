import assert from "node:assert/strict";
import {spawn} from "node:child_process";
import {createInterface} from "node:readline";
import {createClient} from "@connectrpc/connect";
import {createGrpcTransport} from "@connectrpc/connect-node";
import {pastScenario} from "./generated-past.js";
import {queryScenario} from "./generated-query.js";

export async function goReference(vite) {
  const proc=spawn(process.env.PAYDIRT_GO_REFERENCE,[],{stdio:["ignore","pipe","inherit"]});
  const exited=new Promise(resolve=>proc.once("exit",resolve));
  const lines=createInterface({input:proc.stdout});
  try{
    const [addr]=await new Promise((resolve,reject)=>{
      const data=[];lines.on("line",line=>{data.push(line);if(data.length===2)resolve(data);});
      proc.once("error",reject);
      proc.once("exit",code=>reject(Error("Go server exited "+code)));
    });
    const transport=createGrpcTransport({baseUrl:"http://"+addr});
    const {TenantService}=await vite.ssrLoadModule("/upstream/internal/apptest/ts/gen/app/payday/tenant_svc_pb.ts");
    const {RobotService,CellService}=await vite.ssrLoadModule("/upstream/internal/apptest/ts/gen/app/robot_svc_pb.ts");
    const options={headers:{authorization:"Plain @acme/admin"}};
    const acme=await createClient(TenantService,transport).get({ref:{key:{case:"alias",value:"acme"}}},options);
    const report=await queryScenario(createClient(RobotService,transport),acme.id,options);
    assert.deepEqual(report.alias,["query-a","query-b","query-c"]);
    assert.deepEqual(report.removal,[[false,"/app.RobotService/Erase"]]);
    const owned=async name=>createClient((await vite.ssrLoadModule("/upstream/internal/apptest/ts/gen/app/payday/"+name.toLowerCase()+"_svc_pb.ts"))[name+"Service"],transport);
    report.past=await pastScenario({robot:createClient(RobotService,transport),cell:createClient(CellService,transport),event:await owned("Event"),fact:await owned("Fact")},acme.id,options);
    return report;
  }finally{lines.close();proc.kill();await exited;}
}
