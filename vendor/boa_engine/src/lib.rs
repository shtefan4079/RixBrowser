//! Boa's **`boa_engine`** crate implements ECMAScript's standard library of builtin objects
//! and an ECMAScript context, bytecompiler, and virtual machine for code execution.
//!
//! # Example usage
//!
//! You can find multiple examples of the usage of Boa in the [`boa_examples`][examples] crate. In
//! order to use Boa in your project, you will need to add the `boa_engine` crate to your
//! `Cargo.toml` file. You will need to use a [`Source`] structure to handle the JavaScript code
//! to execute, and a [`Context`] structure to execute the code:
//!
//! ```
//! use boa_engine::{Context, Source};
//!
//! let js_code = r#"
//!     let two = 1 + 1;
//!     let definitely_not_four = two + "2";
//!
//!     definitely_not_four
//! "#;
//!
//! // Instantiate the execution context
//! let mut context = Context::default();
//!
//! // Parse the source code
//! match context.eval(Source::from_bytes(js_code)) {
//!     Ok(res) => {
//!         println!(
//!             "{}",
//!             res.to_string(&mut context).unwrap().to_std_string_escaped()
//!         );
//!     }
//!     Err(e) => {
//!         // Pretty print the error
//!         eprintln!("Uncaught {e}");
//!         # panic!("There was an error in boa_engine's introduction example.");
//!     }
//! };
//! ```
//!
//! # Crate Features
//!
//!  - **serde** - Enables serialization and deserialization of the AST (Abstract Syntax Tree).
//!  - **profiler** - Enables profiling with measureme (this is mostly internal).
//!  - **intl** - Enables `boa`'s [ECMA-402 Internationalization API][ecma-402] (`Intl` object)
//!
//! [ecma-402]: https://tc39.es/ecma402
//! [examples]: https://github.com/boa-dev/boa/tree/main/examples
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/boa-dev/boa/main/assets/logo_black.svg",
    html_favicon_url = "https://raw.githubusercontent.com/boa-dev/boa/main/assets/logo_black.svg"
)]
#![cfg_attr(test, allow(clippy::needless_raw_string_hashes))] // Makes strings a bit more copy-pastable
#![cfg_attr(not(test), forbid(clippy::unwrap_used))]
#![allow(
    // Currently throws a false positive regarding dependencies that are only used in benchmarks.
    unused_crate_dependencies,
    clippy::module_name_repetitions,
    clippy::redundant_pub_crate,
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    clippy::missing_errors_doc,
    clippy::let_unit_value,
    clippy::option_if_let_else,

    // It may be worth to look if we can fix the issues highlighted by these lints.
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,

    // Add temporarily - Needs addressing
    clippy::missing_panics_doc,
)]

extern crate self as boa_engine;
#[cfg(not(target_has_atomic = "ptr"))]
compile_error!("Boa requires a lock free `AtomicUsize` in order to work properly.");

pub use boa_ast as ast;
pub use boa_gc as gc;
pub use boa_interner as interner;
pub use boa_parser as parser;

pub mod bigint;
pub mod builtins;
pub mod bytecompiler;
pub mod class;
pub mod context;
pub mod environments;
pub mod error;
pub mod interop;
pub mod job;
pub mod module;
pub mod native_function;
pub mod object;
/// ECMAScript Abstract Syntax Tree (AST) optimizer.
pub mod optimizer;
pub mod property;
pub mod realm;
pub mod script;
pub mod string;
pub mod symbol;
pub mod value;
pub mod vm;

mod host_defined;
mod sys;

mod spanned_source_text;
use spanned_source_text::SourceText;
pub use spanned_source_text::SpannedSourceText;

/// A convenience module that re-exports the most commonly-used Boa APIs
pub mod prelude {
    pub use crate::{
        bigint::JsBigInt,
        context::Context,
        error::{EngineError, JsError, JsNativeError, JsNativeErrorKind, RuntimeLimitError},
        host_defined::HostDefined,
        interop::{IntoJsFunctionCopied, UnsafeIntoJsFunction},
        module::{IntoJsModule, Module},
        native_function::NativeFunction,
        object::{JsData, JsObject, NativeObject},
        script::Script,
        string::{JsStr, JsString},
        symbol::JsSymbol,
        value::{JsValue, JsVariant, js_object, js_value},
    };
    pub use boa_gc::{Finalize, Trace};
    pub use boa_macros::{JsData, js_str};
    pub use boa_parser::Source;
}

#[doc(inline)]
pub use boa_macros::{boa_class, boa_module, embed_module_inner as __embed_module_inner};

use crate::error::PanicError;
use std::result::Result as StdResult;

// Export things to root level
#[doc(inline)]
pub use prelude::*;

#[doc(inline)]
pub use boa_parser::Source;

/// The result of a Javascript expression is represented like this so it can succeed (`Ok`) or fail (`Err`)
pub type JsResult<T> = StdResult<T, JsError>;

/// Create a [`JsResult`] from a Rust value. This trait is used to
/// convert Rust types to JS types, including [`JsResult`] of
/// Rust values and [`JsValue`]s.
///
/// This trait is implemented for any that can be converted into a [`JsValue`].
pub trait TryIntoJsResult {
    /// Try to convert a Rust value into a `JsResult<JsValue>`.
    ///
    /// # Errors
    /// Any parsing errors that may occur during the conversion, or any
    /// error that happened during the call to a function.
    fn try_into_js_result(self, context: &mut Context) -> JsResult<JsValue>;
}

mod try_into_js_result_impls;

/// A utility trait to make working with function arguments easier.
pub trait JsArgs {
    /// Utility function to `get` a parameter from a `[JsValue]` or default to
    /// `JsValue::undefined()` if `get` returns `None`.
    ///
    /// Call this if you are thinking of calling something similar to
    /// `args.get(n).cloned().unwrap_or_default()` or
    /// `args.get(n).unwrap_or(&undefined)`.
    ///
    /// This returns a reference for efficiency, in case you only need to call methods of `JsValue`.
    fn get_or_undefined(&self, index: usize) -> &JsValue;
}

impl JsArgs for [JsValue] {
    fn get_or_undefined(&self, index: usize) -> &JsValue {
        const UNDEFINED: &JsValue = &JsValue::undefined();
        self.get(index).unwrap_or(UNDEFINED)
    }
}

/// Utility trait to "expect" a `JsResult`, but returning a `PanicError` instead of panicking.
#[allow(dead_code)]
pub(crate) trait JsExpect<V> {
    /// "expects" a `JsResult`, wrapping the error with a `PanicError`.
    fn js_expect<S: Into<Box<str>>>(self, msg: S) -> StdResult<V, PanicError>;
}

impl<V> JsExpect<V> for JsResult<V> {
    fn js_expect<S: Into<Box<str>>>(self, msg: S) -> StdResult<V, PanicError> {
        self.map_err(|err| PanicError::new(msg).with_source(err))
    }
}

impl<V> JsExpect<V> for Option<V> {
    fn js_expect<S: Into<Box<str>>>(self, msg: S) -> StdResult<V, PanicError> {
        self.ok_or_else(|| PanicError::new(msg))
    }
}
