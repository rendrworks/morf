//! Arrays, records, data blocks — the types built out of other types.
//!
//! All three are interned and leaked. A shader declares a handful at compile
//! time and the compiler is a short-lived process; giving `Type` a lifetime or
//! an `Rc` to carry them would touch every signature in the crate to save a few
//! dozen bytes that are freed when the process ends anyway.

use crate::types::Type;

/// An array's element type and length.
///
/// Leaked rather than boxed. A shader declares a handful of arrays at compile
/// time and the compiler is a short-lived process; giving `Type` a lifetime or
/// an `Rc` to carry this would touch every signature in the crate to save a few
/// dozen bytes that are freed when the process ends anyway.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ArrayType {
    pub element: Type,
    pub length: u32,
}

impl Type {
    /// Interns an array type.
    pub fn array(element: Type, length: u32) -> Self {
        Self::Array(Box::leak(Box::new(ArrayType { element, length })))
    }

    /// The element type, if this is an array.
    pub fn element(self) -> Option<Type> {
        match self {
            Self::Array(array) => Some(array.element),
            _ => None,
        }
    }

    pub fn is_array(self) -> bool {
        matches!(self, Self::Array(_))
    }
}

/// A record's fields, sorted by name.
///
/// Sorted so that `{a = 1, b = 2}` and `{b = 2, a = 1}` are one type: a Lua
/// table has no order of its own, and two records that differ only in the order
/// they happened to be written are the same record.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct StructType {
    pub fields: Vec<(String, Type)>,
    /// The name this struct is emitted under.
    pub name: String,
}

impl Type {
    /// Interns a record type, reusing one already seen with the same shape.
    pub fn record(mut fields: Vec<(String, Type)>) -> Self {
        fields.sort_by(|left, right| left.0.cmp(&right.0));
        let mut seen = RECORDS.lock().expect("the record table is not poisoned");
        if let Some(found) = seen.iter().find(|known| known.fields == fields) {
            return Self::Struct(found);
        }
        let name = format!("MorfRecord{}", seen.len());
        let interned: &'static StructType = Box::leak(Box::new(StructType { fields, name }));
        seen.push(interned);
        Self::Struct(interned)
    }

    /// Every record type interned so far, in the order they must be declared.
    pub fn records() -> Vec<&'static StructType> {
        RECORDS
            .lock()
            .expect("the record table is not poisoned")
            .clone()
    }

    /// A read-only data block of this element type and length.
    pub fn data(element: Type, length: u32) -> Self {
        Self::Data(Box::leak(Box::new(ArrayType { element, length })))
    }

    /// The element type and length, if this is a data block.
    pub fn data_shape(self) -> Option<(Type, u32)> {
        match self {
            Self::Data(array) => Some((array.element, array.length)),
            _ => None,
        }
    }

    pub fn is_record(self) -> bool {
        matches!(self, Self::Struct(_))
    }

    /// The type of a named field, if this is a record with one.
    pub fn field(self, name: &str) -> Option<Type> {
        match self {
            Self::Struct(record) => record
                .fields
                .iter()
                .find(|(field, _)| field == name)
                .map(|(_, ty)| *ty),
            _ => None,
        }
    }
}

/// Records interned for the life of the process.
///
/// A shader declares a handful and the compiler is short-lived, so leaking is
/// the cheap answer — the same trade as [`ArrayType`], and for the same reason:
/// giving `Type` a lifetime would touch every signature in the crate.
static RECORDS: std::sync::Mutex<Vec<&'static StructType>> = std::sync::Mutex::new(Vec::new());
