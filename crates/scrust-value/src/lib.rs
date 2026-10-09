//! Scratch 3 value model with exact JavaScript semantics.
//!
//! Everything in generated code and the runtime goes through this crate when
//! touching values: `Value`, `JsStr`, the casts, comparison and math.

pub mod jsstr;
mod lower;
pub mod num;
pub mod ops;
pub mod value;

pub use jsstr::{FlatRef, JsStr};
pub use num::{is_js_whitespace, js_num_to_str, js_number_to_string, js_round, js_string_to_number, js_to_fixed10_parse, js_to_fixed_string};
pub use ops::MathOp;
pub use value::{
    compare, compare_str_ci, equals, greater_than, is_int, is_whitespace, less_than, num_eq, num_gt, num_lt, num_to_list_index, to_boolean,
    to_list_index, to_number, to_string, ListIndex, ToBool, ToNum, ToStr, ToValue, Value,
};
