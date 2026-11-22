use crate::Result;
use std::{fs, path::Path};

static FILES: include_dir::Dir = include_dir::include_dir!("$CARGO_MANIFEST_DIR/templates/sandbox");
pub fn init(project: &Path) -> Result<()> {
    let lock = fs::File::open(project.join("buf.yaml"))?;
    lock.lock()?;
    let dest = project.join("sandbox");
    if dest.try_exists()? {
        if !dest.join(".paydirt-sandbox").is_file() {
            return Err("sandbox directory already exists".into());
        }
        return Ok(());
    }
    let stage = tempfile::tempdir_in(project)?;
    FILES.extract(stage.path())?;
    fs::write(
        stage.path().join(".paydirt-sandbox"),
        "Paydirt application sandbox\n",
    )?;
    fs::rename(stage.path(), dest)?;
    Ok(())
}
