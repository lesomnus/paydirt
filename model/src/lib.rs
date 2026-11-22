//! Relational schema shared by code generation, migrations, and the Diesel store.
use heck::ToSnakeCase;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt::Write};

pub type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Model {
    pub version: u32,
    pub entities: Vec<Entity>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Entity {
    pub name: String,
    pub key: String,
    pub fields: Vec<Field>,
    pub indexes: Vec<Index>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Field {
    pub name: String,
    pub json: String,
    pub number: u32,
    pub kind: String,
    #[serde(default)]
    pub collection: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub inverse: String,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default)]
    pub immutable: bool,
    #[serde(default)]
    pub unique: bool,
    pub default: Option<String>,
    #[serde(default)]
    pub version: bool,
    #[serde(default)]
    pub erased: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Index {
    pub name: String,
    pub fields: Vec<String>,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub active_only: bool,
    #[serde(default)]
    pub hidden: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Postgres,
    Sqlite,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scalar {
    Binary,
    Bool,
    Integer,
    BigInt,
    Double,
    Text,
}
impl Scalar {
    pub fn sql(self, dialect: Dialect) -> &'static str {
        match self {
            Self::Binary if dialect == Dialect::Postgres => "BYTEA",
            Self::Binary => "BLOB",
            Self::Bool => "BOOLEAN",
            Self::Integer => "INTEGER",
            Self::BigInt => "BIGINT",
            Self::Double => "DOUBLE PRECISION",
            Self::Text => "TEXT",
        }
    }
    pub fn diesel(self) -> &'static str {
        match self {
            Self::Binary => "Binary",
            Self::Bool => "Bool",
            Self::Integer => "Integer",
            Self::BigInt => "BigInt",
            Self::Double => "Double",
            Self::Text => "Text",
        }
    }
}
pub fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
// Use field numbers for physical columns. A protobuf field rename retains all its data.
impl Field {
    pub fn is_column(&self) -> bool {
        self.inverse.is_empty() && (self.target.is_empty() || self.collection.is_empty())
    }
    pub fn column(&self) -> String {
        format!("f{}", self.number)
    }
    pub fn scalar(&self) -> Result<Scalar> {
        if !self.collection.is_empty() {
            return Ok(Scalar::Text);
        }
        Ok(match self.kind.as_str() {
            "BYTES" | "UUID" => Scalar::Binary,
            "BOOL" => Scalar::Bool,
            "INT32" | "SINT32" | "SFIXED32" => Scalar::Integer,
            "INT64" | "SINT64" | "SFIXED64" | "UINT32" | "FIXED32" => Scalar::BigInt,
            "FLOAT" | "DOUBLE" => Scalar::Double,
            // UInt64 uses fixed-width decimals. Both engines sort the entire unsigned range.
            "UINT64" | "FIXED64" | "ENUM" | "TIME" | "STRING" | "JSON" | "MESSAGE" => Scalar::Text,
            other => return Err(format!("unsupported ORM field type {other}")),
        })
    }
}
fn sql_name(s: &str) -> String {
    if s.len() <= 63 {
        return s.into();
    }
    let hash = s.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    format!("{}_{hash:016x}", &s[..s.floor_char_boundary(46)])
}
impl Entity {
    pub fn table(&self) -> String {
        sql_name(&format!("pd_{}", self.name.replace('.', "__")))
    }
    pub fn managed_tables(&self) -> Vec<String> {
        let table = self.table();
        let mut names = vec![table.clone()];
        names.extend(
            self.fields
                .iter()
                .filter(|f| !f.is_column() && f.inverse.is_empty())
                .map(|f| sql_name(&format!("{table}__edge_{}", f.number))),
        );
        names
    }
    pub fn field(&self, name: &str) -> Result<&Field> {
        self.fields
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| format!("unknown {} field {name}", self.name))
    }
    pub fn key(&self) -> Result<&Field> {
        self.field(&self.key)
    }
    pub fn erased(&self) -> Option<&Field> {
        self.fields.iter().find(|f| f.erased)
    }
    pub fn indexes(&self) -> Vec<Index> {
        let mut indexes = self.indexes.clone();
        for f in &self.fields {
            if !f.is_column()
                || !f.unique
                || f.name == self.key
                || indexes
                    .iter()
                    .any(|i| i.unique && i.fields.as_slice() == std::slice::from_ref(&f.name))
            {
                continue;
            }
            indexes.push(Index {
                name: format!("field_{}", f.number),
                fields: vec![f.name.clone()],
                unique: true,
                active_only: self.erased().is_some(),
                hidden: false,
            });
        }
        indexes
    }
}
impl Model {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let model: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if model.version != 1 {
            return Err(format!("unsupported storage format {}", model.version));
        }
        Ok(model)
    }
    pub fn entity(&self, name: &str) -> Result<&Entity> {
        self.entities
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| format!("unknown ORM entity {name}"))
    }
    pub fn scalar(&self, f: &Field) -> Result<Scalar> {
        if f.target.is_empty() {
            f.scalar()
        } else {
            self.entity(&f.target)?.key()?.scalar()
        }
    }
    pub fn ddl(&self, dialect: Dialect) -> Result<String> {
        self.migration(None, dialect)
    }
    pub fn migration(&self, previous: Option<&Model>, dialect: Dialect) -> Result<String> {
        if let Some(old) = previous {
            for entity in &old.entities {
                let next = self
                    .entity(&entity.name)
                    .map_err(|_| format!("migration would remove {}", entity.name))?;
                if entity.key()?.number != next.key()?.number {
                    return Err(format!("primary key changed on {}", entity.name));
                }
                for field in &entity.fields {
                    let to = next
                        .fields
                        .iter()
                        .find(|f| f.number == field.number)
                        .ok_or_else(|| {
                            format!("migration would remove {}.{}", entity.name, field.name)
                        })?;
                    if field.kind != to.kind
                        || field.target != to.target
                        || field.nullable != to.nullable
                        || field.collection != to.collection
                        || field.inverse != to.inverse
                        || field.erased != to.erased
                        || field.version != to.version
                    {
                        return Err(format!(
                            "incompatible storage change on {}.{}",
                            entity.name, field.name
                        ));
                    }
                }
            }
        }
        let mut sql = String::new();
        let mut later = String::new();
        let mut names = BTreeSet::new();
        for e in &self.entities {
            let old = previous.and_then(|m| m.entities.iter().find(|other| other.name == e.name));
            let table = e.table();
            if !names.insert(table.to_ascii_lowercase()) {
                return Err(format!("duplicate table {table}"));
            }
            let mut defs = Vec::new();
            for f in &e.fields {
                if old.is_some_and(|ent| ent.fields.iter().any(|v| v.number == f.number)) {
                    continue;
                }
                if !f.is_column() {
                    // Inverse properties read the owning side. A forward collection
                    // uses a relation table whose two references are checked by the DB.
                    if f.inverse.is_empty() {
                        let target = self.entity(&f.target)?;
                        let link = sql_name(&format!("{table}__edge_{}", f.number));
                        if !names.insert(link.to_ascii_lowercase()) {
                            return Err(format!("duplicate table {link}"));
                        }
                        let one_owner = target.fields.iter().any(|r| {
                            r.target == e.name && r.inverse == f.name && r.collection.is_empty()
                        });
                        writeln!(later,"CREATE TABLE {} (owner {} NOT NULL REFERENCES {} ({}), target {} NOT NULL REFERENCES {} ({}), PRIMARY KEY (owner, target){});",
                            quote(&link), self.scalar(e.key()?)?.sql(dialect),quote(&table),quote(&e.key()?.column()),
                            self.scalar(target.key()?)?.sql(dialect),quote(&target.table()),quote(&target.key()?.column()),
                            if one_owner { ", UNIQUE (target)" } else { "" }).unwrap();
                    }
                    continue;
                }
                let col = quote(&f.column());
                let mut def = format!("{col} {}", self.scalar(f)?.sql(dialect));
                if !f.nullable || f.name == e.key {
                    def.push_str(" NOT NULL");
                }
                if f.name == e.key {
                    def.push_str(" PRIMARY KEY");
                }
                if !f.target.is_empty() {
                    let to = self.entity(&f.target)?;
                    if to.fields.iter().any(|r| {
                        r.target == e.name && r.inverse == f.name && r.collection.is_empty()
                    }) {
                        def.push_str(" UNIQUE");
                    }
                    let target = format!(
                        "REFERENCES {} ({})",
                        quote(&to.table()),
                        quote(&to.key()?.column())
                    );
                    if dialect == Dialect::Sqlite {
                        write!(def, " {target}").unwrap();
                    } else {
                        writeln!(
                            later,
                            "ALTER TABLE {} ADD FOREIGN KEY ({col}) {target};",
                            quote(&table)
                        )
                        .unwrap();
                    }
                }
                if old.is_some() {
                    if !f.nullable {
                        let default = match (&*f.collection, f.default.as_deref(), self.scalar(f)?)
                        {
                            ("map", _, _) => "'{}'".into(),
                            ("list", _, _) => "'[]'".into(),
                            ("", Some(""), Scalar::Text) if f.kind == "STRING" => "''".into(),
                            ("", Some(""), Scalar::Bool) => "FALSE".into(),
                            ("", Some(""), Scalar::Integer | Scalar::BigInt | Scalar::Double) => {
                                "0".into()
                            }
                            ("", Some(v), Scalar::Text) if f.kind == "STRING" => {
                                format!("'{}'", v.replace('\'', "''"))
                            }
                            _ => {
                                return Err(format!(
                                    "new required field {}.{} needs a backfill",
                                    e.name, f.name
                                ));
                            }
                        };
                        write!(def, " DEFAULT {default}").unwrap();
                    }
                    writeln!(sql, "ALTER TABLE {} ADD COLUMN {def};", quote(&table)).unwrap();
                } else {
                    defs.push(def);
                }
            }
            if old.is_none() {
                writeln!(
                    sql,
                    "CREATE TABLE {} (\n  {}\n);",
                    quote(&table),
                    defs.join(",\n  ")
                )
                .unwrap();
            }
            let signature = |ent: &Entity, idx: &Index| -> Result<_> {
                Ok((
                    idx.unique,
                    idx.active_only,
                    idx.fields
                        .iter()
                        .map(|f| ent.field(f).map(|c| c.number))
                        .collect::<Result<Vec<_>>>()?,
                ))
            };
            if let Some(old) = old {
                for idx in old.indexes() {
                    let keep = if let Some(new) = e.indexes().iter().find(|i| i.name == idx.name) {
                        signature(old, &idx)? == signature(e, new)?
                    } else {
                        false
                    };
                    if !keep {
                        writeln!(
                            sql,
                            "DROP INDEX {};",
                            quote(&sql_name(&format!("{table}__{}", idx.name)))
                        )
                        .unwrap();
                    }
                }
            }
            for i in e.indexes() {
                if let Some(old) = old
                    && let Some(was) = old.indexes().iter().find(|idx| idx.name == i.name)
                    && signature(old, was)? == signature(e, &i)?
                {
                    continue;
                }

                let name = sql_name(&format!("{table}__{}", i.name));
                if !names.insert(name.to_ascii_lowercase()) {
                    return Err(format!("duplicate index {name}"));
                }
                let cols = i
                    .fields
                    .iter()
                    .map(|n| e.field(n).map(|f| quote(&f.column())))
                    .collect::<Result<Vec<_>>>()?;
                let active = if i.active_only {
                    format!(
                        " WHERE {} IS NULL",
                        quote(&e.erased().ok_or("missing erase field")?.column())
                    )
                } else {
                    String::new()
                };
                writeln!(
                    later,
                    "CREATE {}INDEX {} ON {} ({}){active};",
                    if i.unique { "UNIQUE " } else { "" },
                    quote(&name),
                    quote(&table),
                    cols.join(", ")
                )
                .unwrap();
            }
        }
        sql.push_str(&later);
        Ok(sql)
    }
    pub fn diesel_tables(&self) -> Result<String> {
        let mut out = String::from("// @generated by paydirt.\n");
        let mut modules = BTreeSet::new();
        for e in &self.entities {
            let module = e.table().to_snake_case();
            if !modules.insert(module.clone()) {
                return Err(format!("ambiguous Diesel module {module}"));
            }
            writeln!(
                out,
                "::diesel::table! {{\n    #[sql_name = {:?}]\n    {module} ({}) {{",
                e.table(),
                e.key()?.column()
            )
            .unwrap();
            for f in e.fields.iter().filter(|f| f.is_column()) {
                let t = self.scalar(f)?.diesel();
                let t = if f.nullable {
                    format!("Nullable<{t}>")
                } else {
                    t.to_owned()
                };
                writeln!(
                    out,
                    "        /// {}.{}\n        {} -> {t},",
                    e.name,
                    f.name,
                    f.column()
                )
                .unwrap();
            }
            out.push_str("    }\n}\n");
        }
        Ok(out)
    }
}
