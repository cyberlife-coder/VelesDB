//! The one conversion of a JS argument into JSON data: metadata, filters,
//! compile requests and working contexts.
//!
//! napi's own `serde_json::Value` conversion reads any object's own
//! enumerable properties, so a `Date`, a `Map`, a `Set` or a class instance
//! arrived as `{}` and was stored as an empty object, silently. It also read
//! every number past `u32` as a float. This walk accepts what `JSON.parse`
//! can produce — `null`, booleans, finite numbers, strings, arrays and
//! plain objects (prototype `Object.prototype` or `null`) — and refuses
//! anything else with `INVALID_INPUT`, naming where it sits. A whole number
//! a JS `number` holds exactly (up to `2^53 - 1`) becomes an integer
//! ([`json_number`]), and a `BigInt` in the 64-bit range becomes the same
//! integer, losslessly — the type this binding hands back for an integer
//! past `2^53`, so a value read can be written or filtered on again.
//!
//! As `JSON.stringify` does, only own enumerable string-keyed properties
//! are read (never an inherited one), an `undefined` property is left out
//! and an `undefined` array element becomes `null`.
//!
//! The walk is bounded: at most [`MAX_DEPTH`] levels and [`MAX_VALUES`]
//! values in all, counted before anything is allocated, so an array whose
//! `length` claims billions of elements is refused instead of aborting the
//! process on a failed allocation.

use std::cell::Cell;

use napi::bindgen_prelude::{
    JsObjectValue, JsValue, KeyCollectionMode, KeyConversion, KeyFilter, Object, Unknown,
};
use napi::{Env, ValueType};
use serde_json::{Map, Value};

use crate::convert::json_number;
use crate::error::invalid_input;

/// Nesting beyond this is refused: it is far past any metadata or request
/// shape, and a cyclic object would otherwise recurse forever.
const MAX_DEPTH: usize = 64;

/// Values one argument may hold in all, every level counted — the bound
/// napi's own JSON conversion applies. Far past any metadata, filter or
/// request a caller writes by hand.
const MAX_VALUES: usize = 1_000_000;

/// `value` as JSON data, or `INVALID_INPUT` naming the first part of it
/// JSON cannot hold. `what` names the argument in the message.
///
/// # Errors
///
/// `INVALID_INPUT` for a value that is not JSON data; `INTERNAL` if a napi
/// call fails on a value the walk already inspected.
pub fn to_json(value: &Unknown<'_>, what: &str) -> napi::Result<Value> {
    let env = Env::from_raw(value.value().env);
    let object_prototype = env
        .get_global()?
        .get_named_property::<Unknown>("Object")?
        .coerce_to_object()?
        .get_named_property::<Unknown>("prototype")?;
    Walk {
        env,
        object_prototype,
        budget: Cell::new(MAX_VALUES),
    }
    .value(value, what, 0)
}

/// [`to_json`] for an optional argument: absent, `undefined` and `null`
/// are `None`.
///
/// # Errors
///
/// As [`to_json`].
pub fn to_optional_json(value: Option<&Unknown<'_>>, what: &str) -> napi::Result<Option<Value>> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value.get_type()? {
        ValueType::Undefined | ValueType::Null => Ok(None),
        _ => to_json(value, what).map(Some),
    }
}

struct Walk<'env> {
    env: Env,
    object_prototype: Unknown<'env>,
    /// Values still allowed before the argument is refused.
    budget: Cell<usize>,
}

impl Walk<'_> {
    /// Charge `count` values to the budget, or refuse the argument.
    fn spend(&self, count: usize, at: &str) -> napi::Result<()> {
        let left = self.budget.get();
        if count > left {
            return Err(invalid_input(format!(
                "{at} holds more than {MAX_VALUES} values in all, past what one argument may carry"
            )));
        }
        self.budget.set(left - count);
        Ok(())
    }

    fn value(&self, value: &Unknown<'_>, at: &str, depth: usize) -> napi::Result<Value> {
        self.spend(1, at)?;
        match value.get_type()? {
            ValueType::Undefined | ValueType::Null => Ok(Value::Null),
            ValueType::Boolean => Ok(Value::Bool(value.coerce_to_bool()?)),
            ValueType::Number => number(value.coerce_to_number()?.get_double()?, at),
            ValueType::BigInt => bigint(value, at),
            ValueType::String => Ok(Value::String(
                value.coerce_to_string()?.into_utf8()?.as_str()?.to_owned(),
            )),
            ValueType::Object => {
                if depth >= MAX_DEPTH {
                    return Err(invalid_input(format!(
                        "{at} nests deeper than {MAX_DEPTH} levels (or refers to itself)"
                    )));
                }
                if value.is_array()? {
                    self.array(&value.coerce_to_object()?, at, depth)
                } else {
                    self.object(value, at, depth)
                }
            }
            other => Err(invalid_input(format!(
                "{at} is a {}, not JSON data",
                type_name(other)
            ))),
        }
    }

    fn array(&self, array: &Object<'_>, at: &str, depth: usize) -> napi::Result<Value> {
        let len = array.get_array_length()?;
        // The claimed length is checked against the budget before anything
        // is allocated or read: `length` is the caller's to set, and a
        // sparse `[]` with `length = 2 ** 32 - 1` costs nothing to build.
        let claimed = usize::try_from(len).unwrap_or(usize::MAX);
        if claimed > self.budget.get() {
            return Err(invalid_input(format!(
                "{at} has {len} elements, more than the {MAX_VALUES} values one argument may carry"
            )));
        }
        let mut items = Vec::with_capacity(claimed);
        for index in 0..len {
            let item = array.get_element::<Unknown>(index)?;
            items.push(self.value(&item, &format!("{at}[{index}]"), depth + 1)?);
        }
        Ok(Value::Array(items))
    }

    fn object(&self, value: &Unknown<'_>, at: &str, depth: usize) -> napi::Result<Value> {
        if let Some(kind) = self.non_plain_kind(value)? {
            return Err(invalid_input(format!(
                "{at} is a {kind}, not a plain object: pass JSON data \
                 (convert a Date with toISOString(), a Map with Object.fromEntries())"
            )));
        }
        let object = value.coerce_to_object()?;
        // Own keys only, as `JSON.stringify` reads them: the default name
        // listing walks the prototype chain, so an enumerable key someone
        // added to `Object.prototype` landed in every stored object.
        let keys = object.get_all_property_names(
            KeyCollectionMode::OwnOnly,
            KeyFilter::Enumerable,
            KeyConversion::NumbersToStrings,
        )?;
        let mut map = Map::new();
        for index in 0..keys.get_array_length()? {
            let key_value = keys.get_element::<Unknown>(index)?;
            if key_value.get_type()? == ValueType::Symbol {
                // `JSON.stringify` skips symbol keys.
                continue;
            }
            let key = key_value
                .coerce_to_string()?
                .into_utf8()?
                .as_str()?
                .to_owned();
            // Read by the key value itself, not by its text: a named lookup
            // goes through a C string, which a key holding NUL cannot be.
            let entry = object.get_property::<Unknown, Unknown>(key_value)?;
            if entry.get_type()? == ValueType::Undefined {
                continue;
            }
            let entry = self.value(&entry, &format!("{at}.{key}"), depth + 1)?;
            map.insert(key, entry);
        }
        Ok(Value::Object(map))
    }

    /// What a non-plain object is, by its brand or its prototype; `None`
    /// for a plain object.
    fn non_plain_kind(&self, value: &Unknown<'_>) -> napi::Result<Option<&'static str>> {
        let brand = if value.is_date()? {
            Some("Date")
        } else if value.is_promise()? {
            Some("Promise")
        } else if value.is_buffer()? || value.is_typedarray()? || value.is_dataview()? {
            Some("binary view")
        } else if value.is_arraybuffer()? {
            Some("ArrayBuffer")
        } else if value.is_error()? {
            Some("Error")
        } else {
            None
        };
        if brand.is_some() {
            return Ok(brand);
        }
        let prototype = value.coerce_to_object()?.get_prototype()?;
        if prototype.get_type()? == ValueType::Null
            || self.env.strict_equals(prototype, self.object_prototype)?
        {
            Ok(None)
        } else {
            Ok(Some("Map, Set, class instance or other non-plain object"))
        }
    }
}

/// A JS number as JSON ([`json_number`]); `NaN` and the infinities are
/// refused.
fn number(n: f64, at: &str) -> napi::Result<Value> {
    json_number(n)
        .map(Value::Number)
        .ok_or_else(|| invalid_input(format!("{at} is {n}, which JSON cannot hold")))
}

/// A `BigInt` as the JSON integer it holds, read through its decimal text
/// so no digit is lost; one outside `i64::MIN..=u64::MAX`, the integers
/// JSON metadata holds, is refused.
fn bigint(value: &Unknown<'_>, at: &str) -> napi::Result<Value> {
    let text = value.coerce_to_string()?.into_utf8()?.as_str()?.to_owned();
    text.parse::<u64>()
        .map(Value::from)
        .or_else(|_| text.parse::<i64>().map(Value::from))
        .map_err(|_| {
            invalid_input(format!(
                "{at} is {text}n, outside the 64-bit integer range ({}..={}) JSON metadata holds",
                i64::MIN,
                u64::MAX
            ))
        })
}

fn type_name(kind: ValueType) -> &'static str {
    match kind {
        ValueType::Function => "function",
        ValueType::Symbol => "symbol",
        ValueType::External => "native external",
        _ => "value of an unknown kind",
    }
}
