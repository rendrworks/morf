//! Arrays and indexing.
//!
//! A Lua list is a fixed-length array, which is what it looks like and what a
//! palette or a convolution kernel wants to be. The length is part of the type
//! because WGSL's is — a shader has no way to ask how long one is at run time.

use luna::compiler::parser::TableConstructor;

use crate::ir::*;
use crate::lower::{Lowerer, Name};
use crate::types::*;

impl Lowerer<'_> {
    /// `{1.0, 2.0, 3.0}` — a Lua list as a fixed-length array.
    ///
    /// Only the list form. A Lua table with named keys is a record, and this
    /// language has no struct for it to become, so it is refused by name rather
    /// than silently treated as a list of its values in whatever order the
    /// table happened to iterate.
    pub(crate) fn array(&mut self, table: &TableConstructor<Name>, line: u32) -> Expr {
        use luna::compiler::parser::ConstructorField;
        if table.fields.is_empty() {
            self.error_note(
                line,
                "an empty table has no type",
                "a shader array's length and element type are part of what it is",
            );
            return Expr::poison();
        }
        let mut elements = Vec::with_capacity(table.fields.len());
        for field in &table.fields {
            match field {
                ConstructorField::Array(value) => {
                    elements.push(Lowerer::commit(self.expression(value, line)));
                }
                ConstructorField::Record(..) => {
                    self.error_note(
                        line,
                        "a shader table is a list, not a record",
                        "write `{1.0, 2.0}`; named fields would need a struct",
                    );
                    return Expr::poison();
                }
            }
        }
        let element = elements[0].ty();
        if element.is_poison() {
            return Expr::poison();
        }
        for value in &elements[1..] {
            if !value.ty().fits(element) {
                self.error_note(
                    line,
                    format!("this list mixes {element} and {}", value.ty()),
                    "every element of a shader array is the same type",
                );
                return Expr::poison();
            }
        }
        let ty = Type::array(element, elements.len() as u32);
        Expr::Array { ty, elements }
    }

    /// `a[i]` — an array element, a vector component, or a matrix column.
    pub(crate) fn index(&mut self, value: Expr, index: Expr, line: u32) -> Expr {
        let source = value.ty();
        if source.is_poison() || index.ty().is_poison() {
            return Expr::poison();
        }
        if !index.ty().is_integer() && index.ty() != Type::F32 {
            self.error(
                line,
                format!("an index must be a number, not {}", index.ty()),
            );
            return Expr::poison();
        }
        let Some(ty) = source
            .element()
            .or_else(|| source.column())
            .or_else(|| source.is_vector().then_some(Type::F32))
        else {
            self.error_note(
                line,
                format!("{source} cannot be indexed"),
                "arrays, vectors and matrices can; a number cannot",
            );
            return Expr::poison();
        };
        // WGSL indexes with a whole number, and a `f32` index is a mistake
        // worth naming rather than rounding silently.
        let index = if index.ty() == Type::F32 {
            self.error_note(
                line,
                "an index must be a whole number",
                "convert it: `a[i32(n)]`",
            );
            return Expr::poison();
        } else {
            index
        };
        Expr::Index {
            ty,
            value: Box::new(value),
            index: Box::new(index),
        }
    }
}
