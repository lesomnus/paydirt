use crate::{
    operator::{Result, invoke},
    tree::{Command, Tree},
};
use serde_json::json;

pub fn configure(tree: &mut Tree) -> Result<()> {
    tree.mount("inventory", "app")?;
    tree.mount("catalog", "shared")?;
    tree.alias("inventory/view", "cell/get")?;
    tree.hide("inventory/view")?;
    tree.add(
        "spotlight",
        Command::Custom(|client, config, _| {
            Box::pin(async move {
                let rows =
                    invoke(client, config, "shared.Thing", "List", json!({"size":1})).await?;
                println!("{}", rows[0]["items"].as_array().map(Vec::len).unwrap_or(0));
                Ok(())
            })
        }),
    )?;
    tree.replace(
        "inventory/cell/get",
        Command::Custom(|_, _, _| {
            Box::pin(async {
                println!("replaced");
                Ok(())
            })
        }),
    )?;
    tree.formats.insert("label".into(), |v| {
        Ok(format!("label={}", v["alias"].as_str().unwrap_or("")))
    });
    tree.columns.insert(
        "app.Cell".into(),
        vec![("NAME", "/alias"), ("RESOURCE", "/id")],
    );
    Ok(())
}
