"""Add the Go reference entities and the application-owned Move layer to an existing CLI starter."""
import re
import sys
from pathlib import Path
from shutil import copyfile

repo = Path(__file__).resolve().parents[1]
app = Path(sys.argv[1])
proto = repo / "web/upstream/internal/apptest/proto"
(app / "proto/app/cell.proto").unlink(missing_ok=True)
for rel in ["app/computed_reading.proto", "ext/app/computed_reading_svc.ext.proto", "shared/thing.proto", "app/robot.proto", "ext/app/robot_svc.ext.proto"]:
    dest = app / "proto" / rel
    dest.parent.mkdir(exist_ok=True, parents=True)
    copyfile(proto / rel, dest)
# The reference file also declares metering and other unrelated entities.
robot = app / "proto/app/robot.proto"
source = robot.read_text()
header = source.split("// This package", 1)[0]
entities = [re.search(rf"^message {name} \{{.*?^\}}", source, re.M | re.S).group() for name in ["Cell", "Robot", "Pairing", "Reading","RobotMoved"]]
robot.write_text(header + "option (payday.app) = {accounts: {field: {name: \"account\", number: 12}} past: {event:17, fact:18} entitlements:{domain:43} meters:[{key:\"apptest.robots\" unit:\"{robot}\" count:{of:\"Robot\"}},{key:\"apptest.seats\" unit:\"{seat}\" count:{of:\"Holder\"}},{key:\"apptest.robots.attested\" unit:\"{robot}\" count:{of:\"Robot\" where:[{field:{name:\"date_attested\" number:31} set:true}]}}] notify:{notification:40 receipt:41 accounts:{} keep:{days:90 retry_days:1}}};\n" + "\n\n".join(entities) + "\n")
for rel in ["ext/app/payday/holder_svc.ext.proto", "ext/payday/holder.ext.proto","app/profile.proto"]:
    dest=app / "proto" / rel
    dest.parent.mkdir(exist_ok=True,parents=True)
    copyfile(proto / rel,dest)
copyfile(repo / "tests/fleet/readings.rs", app / "src/readings.rs")
copyfile(repo / "tests/fleet/robot.rs", app / "src/robot.rs")
lib = app / "src/lib.rs"
source = lib.read_text()
source = "mod readings;\nmod robot;\n" + source.replace("struct App(String);", "struct App(String, storage::Next);")
source = source.replace("impl storage::Layers for App {", """impl storage::Layers for App {
        fn app_computed_reading(&self) -> impl services::app::ComputedReadingService {
            readings::PublicReadings
        }
        fn app_robot(&self, next: impl services::app::RobotService) -> impl services::app::RobotService {
            services::app::RobotServiceOverlay { next, layer: robot::Fleet(self.1.clone()) }
        }""")
source = source.replace("database, &App(name)", "database.clone(), &App(name, storage::Next(database))")
# Test the transaction boundary with a second independent user layer.
copyfile(repo / "tests/fleet/pair.rs", app / "src/pair.rs")
overlay = app / "proto/ext/app/robot_svc.ext.proto"
overlay.write_text(overlay.read_text().replace("service RobotService {", "service RobotService {\n  rpc RenamePair(RobotRenamePairRequest) returns (Robot);\n  rpc Allowance(RobotAllowanceRequest) returns (RobotAllowanceResponse);") + "\n" + (repo / "tests/fleet/pair.proto").read_text())
source = "mod pair;\n" + source.replace("services::app::RobotServiceOverlay { next, layer: robot::Fleet(self.1.clone()) }", """let core=services::app::RobotServiceOverlay { next, layer: robot::Fleet(self.1.clone()) };
            services::app::RobotServiceOverlay { next:core, layer:pair::Pair(self.1.clone()) }""")
lib.write_text(source)

# The upstream test page authenticates with Plain @acme/admin. Only this reference fixture trusts it.
worker=app / "src/worker.rs"
source=worker.read_text()
needle=".allow_general_writes(general_writes);"
if needle in source:
    worker.write_text(source.replace(needle, """.allow_general_writes(general_writes)
        .with_resolver(|ctx| Ok((ctx.headers().get("authorization").and_then(|v|v.to_str().ok())==Some("Plain @acme/admin")).then(||"sdk-admin".to_owned())));"""))
