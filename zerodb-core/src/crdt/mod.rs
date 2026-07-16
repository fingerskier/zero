pub mod lww;

use serde::{Deserialize, Serialize};

/// Scalar value type for CRDT payloads.
///
/// Does not implement `Eq` because `f64` has NaN.
/// CRDTs compare timestamps for ordering, not values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ScalarValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
}
