"""Add the Go reference entities and the application-owned Move layer to an existing CLI starter."""
import re
import sys
from pathlib import Path
from shutil import copyfile

repo = Path(__file__).resolve().parents[1]
app = Path(sys.argv[1])
proto = repo / "web/upstream/internal/apptest/proto"
(app / "proto/app/cell.proto").unlink(missing_ok=True)
for rel in ["shared/thing.proto", "app/robot.proto", "ext/app/robot_svc.ext.proto"]:
    dest = app / "proto" / rel
    dest.parent.mkdir(exist_ok=True, parents=True)
    copyfile(proto / rel, dest)
# The reference file also declares metering and other unrelated entities.
robot = app / "proto/app/robot.proto"
source = robot.read_text()
header = source.split("// This package", 1)[0]
entities = [re.search(rf"^message {name} \{{.*?^\}}", source, re.M | re.S).group() for name in ["Cell", "Robot"]]
robot.write_text(header + "option (payday.app) = {};\n" + "\n\n".join(entities) + "\n")
copyfile(repo / "tests/fleet/robot.rs", app / "src/robot.rs")
lib = app / "src/lib.rs"
source = lib.read_text()
source = "mod robot;\n" + source.replace("struct App(String);", "struct App(String, storage::Next);")
source = source.replace("impl storage::Layers for App {", """impl storage::Layers for App {
        fn app_robot(&self, next: impl services::app::RobotService) -> impl services::app::RobotService {
            services::app::RobotServiceOverlay { next, layer: robot::Fleet(self.1.clone()) }
        }""")
source = source.replace("database, &App(name)", "database.clone(), &App(name, storage::Next(database))")
lib.write_text(source)
