//! Schema IR structural validation and typed parse per doc/SCHEMA.md §2.
//!
//! `parse_ir` accepts a decoded canonical-CBOR value and returns the typed
//! IR or a named validation outcome. Unknown keys are rejected everywhere —
//! evolution goes through `schema_ir_format_version`, and this is what
//! makes `unique` structurally unsmuggleable (DQ-10).

use std::collections::BTreeMap;

use crate::cbor::Cbor;
use thiserror::Error;

pub const SCHEMA_IR_VERSION: u64 = 1;
/// Registry `domain_separation.schema_ir`.
pub const DOMAIN_SCHEMA_IR: &[u8] = b"zerodb-schema-ir-v1";

/// `SchemaId = BLAKE3(domain("schema_ir") ‖ canonical IR bytes)`.
pub fn schema_id(ir_bytes: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN_SCHEMA_IR);
    hasher.update(ir_bytes);
    *hasher.finalize().as_bytes()
}

pub fn crdt_from_name(name: &str) -> Option<u64> {
    match name {
        "lww" => Some(crdt_tag::LWW),
        "gcounter" => Some(crdt_tag::GCOUNTER),
        "pncounter" => Some(crdt_tag::PNCOUNTER),
        "orset" => Some(crdt_tag::ORSET),
        "flag" => Some(crdt_tag::FLAG),
        _ => None,
    }
}

pub fn crdt_name(tag: u64) -> Option<&'static str> {
    match tag {
        crdt_tag::LWW => Some("lww"),
        crdt_tag::GCOUNTER => Some("gcounter"),
        crdt_tag::PNCOUNTER => Some("pncounter"),
        crdt_tag::ORSET => Some("orset"),
        crdt_tag::FLAG => Some("flag"),
        _ => None,
    }
}

pub fn default_value_type(crdt: u64) -> u64 {
    match crdt {
        crdt_tag::GCOUNTER | crdt_tag::PNCOUNTER => value_type::INT,
        crdt_tag::FLAG => value_type::BOOL,
        _ => value_type::TEXT,
    }
}

fn encode_prop(p: &PropDef) -> Cbor {
    Cbor::Map(vec![
        ("crdt".into(), Cbor::Uint(p.crdt)),
        ("encrypted".into(), Cbor::Bool(p.encrypted)),
        ("nullable".into(), Cbor::Bool(p.nullable)),
        ("type".into(), Cbor::Uint(p.value_type)),
    ])
}

fn encode_props(props: &BTreeMap<String, PropDef>) -> Cbor {
    Cbor::Map(
        props
            .iter()
            .map(|(k, p)| (k.clone(), encode_prop(p)))
            .collect(),
    )
}

/// Canonical CBOR encoding of a parsed IR (round-trips through [`parse_ir`]).
pub fn ir_to_cbor(ir: &SchemaIr) -> Cbor {
    let mut top = vec![("v".into(), Cbor::Uint(SCHEMA_IR_VERSION))];
    if let Some(name) = &ir.name {
        top.push(("name".into(), Cbor::Text(name.clone())));
    }
    top.push((
        "nodes".into(),
        Cbor::Map(
            ir.nodes
                .iter()
                .map(|(label, ent)| {
                    (
                        label.clone(),
                        Cbor::Map(vec![("props".into(), encode_props(&ent.props))]),
                    )
                })
                .collect(),
        ),
    ));
    top.push((
        "edges".into(),
        Cbor::Map(
            ir.edges
                .iter()
                .map(|(label, ent)| {
                    let src = match &ent.src {
                        None => Cbor::Null,
                        Some(ls) => Cbor::Array(ls.iter().map(|s| Cbor::Text(s.clone())).collect()),
                    };
                    let dst = match &ent.dst {
                        None => Cbor::Null,
                        Some(ls) => Cbor::Array(ls.iter().map(|s| Cbor::Text(s.clone())).collect()),
                    };
                    (
                        label.clone(),
                        Cbor::Map(vec![
                            ("dst".into(), dst),
                            ("props".into(), encode_props(&ent.props)),
                            ("src".into(), src),
                        ]),
                    )
                })
                .collect(),
        ),
    ));
    Cbor::Map(top)
}

pub fn encode_ir(ir: &SchemaIr) -> Result<Vec<u8>, crate::cbor::CborError> {
    crate::cbor::encode(&ir_to_cbor(ir))
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SchemaError {
    #[error("unknown key in IR map")]
    UnknownKey,
    #[error("unsupported schema_ir_format_version")]
    VersionUnsupported,
    #[error("reserved or unregistered crdt tag")]
    CrdtUnsupported,
    #[error("encrypted is only legal on lww (and mvregister once active)")]
    EncryptedInvalid,
    #[error("crdt/value-type combination is invalid")]
    TypeMismatch,
    #[error("malformed IR structure")]
    Invalid,
}

/// Registry `crdt_tags` active in v0.1 (5–7 reserved for M2).
pub mod crdt_tag {
    pub const LWW: u64 = 0;
    pub const GCOUNTER: u64 = 1;
    pub const PNCOUNTER: u64 = 2;
    pub const ORSET: u64 = 3;
    pub const FLAG: u64 = 4;
}

/// Registry `value_type_tags`.
pub mod value_type {
    pub const ANY_SCALAR: u64 = 0;
    pub const BOOL: u64 = 1;
    pub const INT: u64 = 2;
    pub const FLOAT64: u64 = 3;
    pub const TEXT: u64 = 4;
    pub const BYTES: u64 = 5;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropDef {
    pub crdt: u64,
    pub value_type: u64,
    pub nullable: bool,
    pub encrypted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EntityDef {
    pub props: BTreeMap<String, PropDef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeDef {
    pub props: BTreeMap<String, PropDef>,
    pub src: Option<Vec<String>>,
    pub dst: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaIr {
    pub name: Option<String>,
    pub nodes: BTreeMap<String, EntityDef>,
    pub edges: BTreeMap<String, EdgeDef>,
}

fn entries(v: &Cbor) -> Result<&[(String, Cbor)], SchemaError> {
    match v {
        Cbor::Map(e) => Ok(e),
        _ => Err(SchemaError::Invalid),
    }
}

fn get<'a>(map: &'a [(String, Cbor)], key: &str) -> Option<&'a Cbor> {
    map.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn check_keys(
    map: &[(String, Cbor)],
    allowed: &[&str],
    required: &[&str],
) -> Result<(), SchemaError> {
    for (k, _) in map {
        if !allowed.contains(&k.as_str()) {
            return Err(SchemaError::UnknownKey);
        }
    }
    for r in required {
        if get(map, r).is_none() {
            return Err(SchemaError::Invalid);
        }
    }
    Ok(())
}

fn uint(v: &Cbor) -> Result<u64, SchemaError> {
    match v {
        Cbor::Uint(n) => Ok(*n),
        _ => Err(SchemaError::Invalid),
    }
}

fn boolean(v: &Cbor) -> Result<bool, SchemaError> {
    match v {
        Cbor::Bool(b) => Ok(*b),
        _ => Err(SchemaError::Invalid),
    }
}

fn parse_prop(v: &Cbor) -> Result<PropDef, SchemaError> {
    let map = entries(v)?;
    check_keys(
        map,
        &["crdt", "type", "nullable", "encrypted"],
        &["crdt", "type", "nullable", "encrypted"],
    )?;

    let crdt = uint(get(map, "crdt").unwrap())?;
    let value_type = uint(get(map, "type").unwrap())?;
    let nullable = boolean(get(map, "nullable").unwrap())?;
    let encrypted = boolean(get(map, "encrypted").unwrap())?;

    match crdt {
        crdt_tag::LWW
        | crdt_tag::GCOUNTER
        | crdt_tag::PNCOUNTER
        | crdt_tag::ORSET
        | crdt_tag::FLAG => {}
        5..=7 => return Err(SchemaError::CrdtUnsupported), // reserved (M2)
        _ => return Err(SchemaError::CrdtUnsupported),
    }
    if value_type > value_type::BYTES {
        return Err(SchemaError::Invalid);
    }
    if encrypted && crdt != crdt_tag::LWW {
        return Err(SchemaError::EncryptedInvalid);
    }
    match crdt {
        crdt_tag::GCOUNTER | crdt_tag::PNCOUNTER if value_type != value_type::INT => {
            return Err(SchemaError::TypeMismatch);
        }
        crdt_tag::FLAG if value_type != value_type::BOOL => return Err(SchemaError::TypeMismatch),
        _ => {}
    }

    Ok(PropDef {
        crdt,
        value_type,
        nullable,
        encrypted,
    })
}

fn parse_props(v: &Cbor) -> Result<BTreeMap<String, PropDef>, SchemaError> {
    let mut props = BTreeMap::new();
    for (path, def) in entries(v)? {
        props.insert(path.clone(), parse_prop(def)?);
    }
    Ok(props)
}

fn parse_labels(v: &Cbor) -> Result<Option<Vec<String>>, SchemaError> {
    match v {
        Cbor::Null => Ok(None),
        Cbor::Array(items) => items
            .iter()
            .map(|i| match i {
                Cbor::Text(s) => Ok(s.clone()),
                _ => Err(SchemaError::Invalid),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        _ => Err(SchemaError::Invalid),
    }
}

/// Validate and parse a decoded IR value (doc/SCHEMA.md §2).
pub fn parse_ir(ir: &Cbor) -> Result<SchemaIr, SchemaError> {
    let top = entries(ir)?;
    check_keys(
        top,
        &["v", "name", "nodes", "edges"],
        &["v", "nodes", "edges"],
    )?;

    if uint(get(top, "v").unwrap())? != SCHEMA_IR_VERSION {
        return Err(SchemaError::VersionUnsupported);
    }
    let name = match get(top, "name") {
        None => None,
        Some(Cbor::Text(s)) => Some(s.clone()),
        Some(_) => return Err(SchemaError::Invalid),
    };

    let mut nodes = BTreeMap::new();
    for (label, def) in entries(get(top, "nodes").unwrap())? {
        let map = entries(def)?;
        check_keys(map, &["props"], &["props"])?;
        nodes.insert(
            label.clone(),
            EntityDef {
                props: parse_props(get(map, "props").unwrap())?,
            },
        );
    }

    let mut edges = BTreeMap::new();
    for (label, def) in entries(get(top, "edges").unwrap())? {
        let map = entries(def)?;
        check_keys(map, &["props", "src", "dst"], &["props", "src", "dst"])?;
        edges.insert(
            label.clone(),
            EdgeDef {
                props: parse_props(get(map, "props").unwrap())?,
                src: parse_labels(get(map, "src").unwrap())?,
                dst: parse_labels(get(map, "dst").unwrap())?,
            },
        );
    }

    Ok(SchemaIr { name, nodes, edges })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop(crdt: u64, vt: u64, encrypted: bool) -> Cbor {
        Cbor::Map(vec![
            ("crdt".into(), Cbor::Uint(crdt)),
            ("type".into(), Cbor::Uint(vt)),
            ("nullable".into(), Cbor::Bool(false)),
            ("encrypted".into(), Cbor::Bool(encrypted)),
        ])
    }

    fn ir_with_prop(p: Cbor) -> Cbor {
        Cbor::Map(vec![
            ("v".into(), Cbor::Uint(1)),
            (
                "nodes".into(),
                Cbor::Map(vec![(
                    "N".into(),
                    Cbor::Map(vec![("props".into(), Cbor::Map(vec![("p".into(), p)]))]),
                )]),
            ),
            ("edges".into(), Cbor::Map(vec![])),
        ])
    }

    #[test]
    fn valid_ir_parses() {
        assert!(parse_ir(&ir_with_prop(prop(0, 4, false))).is_ok());
    }

    #[test]
    fn encode_ir_round_trips() {
        let ir = parse_ir(&ir_with_prop(prop(0, 4, false))).unwrap();
        let bytes = encode_ir(&ir).unwrap();
        let decoded = crate::cbor::decode(&bytes).unwrap();
        assert_eq!(parse_ir(&decoded).unwrap(), ir);
        assert_eq!(schema_id(&bytes), schema_id(&encode_ir(&ir).unwrap()));
    }

    #[test]
    fn named_outcomes() {
        // unique smuggled into a PropDef → UnknownKey
        let mut smuggled = prop(0, 4, false);
        if let Cbor::Map(e) = &mut smuggled {
            e.push(("unique".into(), Cbor::Bool(true)));
        }
        assert_eq!(
            parse_ir(&ir_with_prop(smuggled)),
            Err(SchemaError::UnknownKey)
        );

        assert_eq!(
            parse_ir(&ir_with_prop(prop(7, 4, false))),
            Err(SchemaError::CrdtUnsupported)
        );
        assert_eq!(
            parse_ir(&ir_with_prop(prop(1, 2, true))),
            Err(SchemaError::EncryptedInvalid)
        );
        assert_eq!(
            parse_ir(&ir_with_prop(prop(1, 4, false))),
            Err(SchemaError::TypeMismatch)
        );
        assert_eq!(
            parse_ir(&ir_with_prop(prop(4, 2, false))),
            Err(SchemaError::TypeMismatch)
        );
    }
}
