//! This module contains the [`TryFromJs`] trait, and conversions to basic Rust types.

use crate::{Context, JsBigInt, JsNativeError, JsObject, JsResult, JsString, JsValue, js_error};
use boa_string::StaticJsStrings;
use num_bigint::BigInt;
use num_traits::AsPrimitive;

mod collections;
mod tuples;

/// This trait adds a fallible and efficient conversions from a [`JsValue`] to Rust types.
///
/// # Floating-point types
///
/// [`TryFromJs`] is implemented for [`f64`] because JavaScript numbers are IEEE-754
/// double-precision values, so that conversion is exact.
///
/// It is intentionally **not** implemented for [`f32`]. Converting a JS number to
/// `f32` can lose precision, and Rust prefers that loss to be explicit. Convert to
/// [`f64`] first, then cast with `as f32`:
///
/// ```
/// # use boa_engine::{Context, JsResult, JsValue, value::TryFromJs};
/// fn to_f32(value: &JsValue, context: &mut Context) -> JsResult<f32> {
///     Ok(f64::try_from_js(value, context)? as f32)
/// }
/// ```
///
/// For derived structs with an `f32` field, use
/// `#[boa(from_js_with = "...")]` to supply a custom converter.
///
/// If you want JavaScript-style coercion (for example accepting numeric strings),
/// use [`JsValue::to_f32`] instead. That path is also lossy by design.
pub trait TryFromJs: Sized {
    /// This function tries to convert a JavaScript value into `Self`.
    fn try_from_js(value: &JsValue, context: &mut Context) -> JsResult<Self>;
}

impl JsValue {
    /// This function is the inverse of [`TryFromJs`]. It tries to convert a [`JsValue`] to a given
    /// Rust type.
    pub fn try_js_into<T>(&self, context: &mut Context) -> JsResult<T>
    where
        T: TryFromJs,
    {
        T::try_from_js(self, context)
    }
}

impl TryFromJs for bool {
    fn try_from_js(value: &JsValue, _context: &mut Context) -> JsResult<Self> {
        if let Some(b) = value.as_boolean() {
            Ok(b)
        } else {
            Err(JsNativeError::typ()
                .with_message("cannot convert value to a boolean")
                .into())
        }
    }
}

impl TryFromJs for () {
    fn try_from_js(_value: &JsValue, _context: &mut Context) -> JsResult<Self> {
        Ok(())
    }
}

impl TryFromJs for String {
    fn try_from_js(value: &JsValue, _context: &mut Context) -> JsResult<Self> {
        if let Some(s) = value.as_string() {
            s.to_std_string().map_err(|e| {
                JsNativeError::typ()
                    .with_message(format!("could not convert JsString to Rust string: {e}"))
                    .into()
            })
        } else {
            Err(JsNativeError::typ()
                .with_message("cannot convert value to a String")
                .into())
        }
    }
}

impl TryFromJs for JsString {
    fn try_from_js(value: &JsValue, _context: &mut Context) -> JsResult<Self> {
        if let Some(s) = value.as_string() {
            Ok(s.clone())
        } else {
            Err(JsNativeError::typ()
                .with_message("cannot convert value to a JsString")
                .into())
        }
    }
}

impl<T> TryFromJs for Option<T>
where
    T: TryFromJs,
{
    fn try_from_js(value: &JsValue, context: &mut Context) -> JsResult<Self> {
        if value.is_undefined() {
            Ok(None)
        } else {
            Ok(Some(T::try_from_js(value, context)?))
        }
    }
}

impl<T> TryFromJs for Vec<T>
where
    T: TryFromJs,
{
    fn try_from_js(value: &JsValue, context: &mut Context) -> JsResult<Self> {
        let Some(object) = &value.as_object() else {
            return Err(JsNativeError::typ()
                .with_message("cannot convert value to a Vec")
                .into());
        };

        let length = object.get(StaticJsStrings::LENGTH, context)?;
        // If there's no length, return an error.
        if length.is_null_or_undefined() {
            return Err(js_error!(TypeError: "Not an array"));
        }
        let length = length.to_length(context)?;

        let length = match usize::try_from(length) {
            Ok(length) => length,
            Err(e) => {
                return Err(JsNativeError::typ()
                    .with_message(format!("could not convert length to usize: {e}"))
                    .into());
            }
        };
        let mut vec = Vec::with_capacity(length);
        for i in 0..length {
            let value = object.get(i, context)?;
            vec.push(T::try_from_js(&value, context)?);
        }

        Ok(vec)
    }
}

impl TryFromJs for JsObject {
    fn try_from_js(value: &JsValue, _context: &mut Context) -> JsResult<Self> {
        if let Some(o) = value.as_object() {
            Ok(o.clone())
        } else {
            Err(JsNativeError::typ()
                .with_message("cannot convert value to a Object")
                .into())
        }
    }
}

impl TryFromJs for JsBigInt {
    fn try_from_js(value: &JsValue, _context: &mut Context) -> JsResult<Self> {
        if let Some(b) = value.as_bigint() {
            Ok(b.clone())
        } else {
            Err(JsNativeError::typ()
                .with_message("cannot convert value to a BigInt")
                .into())
        }
    }
}

impl TryFromJs for BigInt {
    fn try_from_js(value: &JsValue, _context: &mut Context) -> JsResult<Self> {
        if let Some(b) = value.as_bigint() {
            Ok(b.as_inner().clone())
        } else {
            Err(JsNativeError::typ()
                .with_message("cannot convert value to a BigInt")
                .into())
        }
    }
}

impl TryFromJs for JsValue {
    fn try_from_js(value: &JsValue, _context: &mut Context) -> JsResult<Self> {
        Ok(value.clone())
    }
}

impl TryFromJs for f64 {
    fn try_from_js(value: &JsValue, _context: &mut Context) -> JsResult<Self> {
        if let Some(i) = value.0.as_integer32() {
            Ok(f64::from(i))
        } else if let Some(f) = value.0.as_float64() {
            Ok(f)
        } else {
            Err(JsNativeError::typ()
                .with_message("cannot convert value to a f64")
                .into())
        }
    }
}

fn from_f64<T>(v: f64) -> Option<T>
where
    T: AsPrimitive<f64>,
    f64: AsPrimitive<T>,
{
    if <f64 as AsPrimitive<T>>::as_(v).as_().to_bits() == v.to_bits() {
        return Some(v.as_());
    }
    None
}

macro_rules! impl_try_from_js_integer {
    ( $( $type: ty ),* ) => {
        $(
            impl TryFromJs for $type {
                fn try_from_js(value: &JsValue, _context: &mut Context) -> JsResult<Self> {
                    if let Some(i) = value.as_i32() {
                        i.try_into().map_err(|e| {
                            JsNativeError::typ()
                                .with_message(format!(
                                    concat!("cannot convert value to a ", stringify!($type), ": {}"),
                                    e)
                                )
                                .into()
                        })
                    } else if let Some(f) = value.as_number() {
                        from_f64(f).ok_or_else(|| {
                            JsNativeError::typ()
                                .with_message(concat!("cannot convert value to a ", stringify!($type)))
                                .into()
                        })
                    } else {
                        Err(JsNativeError::typ()
                            .with_message(concat!("cannot convert value to a ", stringify!($type)))
                            .into())
                    }
                }
            }
        )*
    }
}

impl_try_from_js_integer!(i8, u8, i16, u16, i32, u32, i64, u64, usize, i128, u128);
