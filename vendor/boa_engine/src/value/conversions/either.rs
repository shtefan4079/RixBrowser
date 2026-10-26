//! Implementation of [`TryFromJs`] for [`Either`].
//!
//! This will try to deserialize for the [`Either::Left`] type
//! first, and if it fails will try the [`Either::Right`] type.
//!
//! Upon failure of both, the second failure will be returned.
#![cfg(feature = "either")]

use crate::value::TryFromJs;
use boa_engine::{Context, JsResult, JsValue};
use either::Either;

impl<L, R> TryFromJs for Either<L, R>
where
    L: TryFromJs,
    R: TryFromJs,
{
    #[inline]
    fn try_from_js(value: &JsValue, context: &mut Context) -> JsResult<Self> {
        L::try_from_js(value, context)
            .map(Self::Left)
            .or_else(|_| R::try_from_js(value, context).map(Self::Right))
    }
}
