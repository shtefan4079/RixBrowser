
use boa_engine::{Context, JsError};
use std::{cell::RefCell, collections::VecDeque};

#[derive(Default)]
struct ReportedExceptions(RefCell<VecDeque<String>>);

pub(crate) fn report(error: JsError, context: &mut Context) {
    report_message(error.to_string(), context);
}

pub(crate) fn report_message(message: String, context: &mut Context) {
    tracing::warn!(%message, "uncaught page exception");
    if context.get_data::<ReportedExceptions>().is_none() {
        context.insert_data(ReportedExceptions::default());
    }
    if let Some(errors) = context.get_data::<ReportedExceptions>() {
        errors.0.borrow_mut().push_back(message);
    }
}

pub(crate) fn take(context: &mut Context) -> Vec<String> {
    context
        .get_data::<ReportedExceptions>()
        .map(|errors| errors.0.borrow_mut().drain(..).collect())
        .unwrap_or_default()
}

pub fn take_reported_exceptions(context: &mut Context) -> Vec<String> {
    take(context)
}
