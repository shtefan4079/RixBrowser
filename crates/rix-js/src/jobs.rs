
use boa_engine::builtins::promise::{OperationType, Promise, PromiseState};
use boa_engine::context::HostHooks;
use boa_engine::job::{Job, JobExecutor, NativeJob, SimpleJobExecutor};
use boa_engine::object::builtins::JsPromise;
use boa_engine::{Context, JsObject, JsResult, JsValue};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

const MAX_TRACKED_PROMISE_REJECTIONS: usize = 256;

#[derive(Clone)]
pub(crate) struct PromiseRejectionRecord {
    pub(crate) promise: JsObject<Promise>,
    pub(crate) reason: JsValue,
}

#[derive(Default)]
pub(crate) struct PromiseRejectionTracker {
    pending_unhandled: VecDeque<PromiseRejectionRecord>,
    reported_unhandled: VecDeque<PromiseRejectionRecord>,
    pending_handled: VecDeque<PromiseRejectionRecord>,
}

impl PromiseRejectionTracker {
    fn track_rejection(&mut self, promise: &JsObject<Promise>) {
        let Ok(promise_state) = JsPromise::from_object(promise.clone().upcast()) else {
            return;
        };
        let PromiseState::Rejected(reason) = promise_state.state() else {
            return;
        };
        if self.is_known(promise) {
            return;
        }
        push_bounded(
            &mut self.pending_unhandled,
            PromiseRejectionRecord {
                promise: promise.clone(),
                reason,
            },
        );
    }

    fn track_handler(&mut self, promise: &JsObject<Promise>) {
        if let Some(index) = self
            .pending_unhandled
            .iter()
            .position(|record| record.promise == *promise)
        {
            self.pending_unhandled.remove(index);
            return;
        }
        if let Some(index) = self
            .reported_unhandled
            .iter()
            .position(|record| record.promise == *promise)
        {
            if let Some(record) = self.reported_unhandled.remove(index) {
                push_bounded(&mut self.pending_handled, record);
            }
        }
    }

    fn is_known(&self, promise: &JsObject<Promise>) -> bool {
        self.pending_unhandled
            .iter()
            .chain(self.reported_unhandled.iter())
            .chain(self.pending_handled.iter())
            .any(|record| record.promise == *promise)
    }

    pub(crate) fn has_pending_events(&self) -> bool {
        !self.pending_unhandled.is_empty() || !self.pending_handled.is_empty()
    }

    pub(crate) fn take_pending_unhandled(&mut self) -> Vec<PromiseRejectionRecord> {
        let pending = std::mem::take(&mut self.pending_unhandled);
        for record in &pending {
            push_bounded(&mut self.reported_unhandled, record.clone());
        }
        pending.into_iter().collect()
    }

    pub(crate) fn take_pending_handled(&mut self) -> Vec<PromiseRejectionRecord> {
        std::mem::take(&mut self.pending_handled)
            .into_iter()
            .collect()
    }
}

fn push_bounded<T>(queue: &mut VecDeque<T>, value: T) {
    if queue.len() == MAX_TRACKED_PROMISE_REJECTIONS {
        queue.pop_front();
    }
    queue.push_back(value);
}

pub(crate) struct PromiseRejectionHooks {
    tracker: Rc<RefCell<PromiseRejectionTracker>>,
}

impl PromiseRejectionHooks {
    pub(crate) fn new(tracker: Rc<RefCell<PromiseRejectionTracker>>) -> Self {
        Self { tracker }
    }
}

impl HostHooks for PromiseRejectionHooks {
    fn promise_rejection_tracker(
        &self,
        promise: &JsObject<Promise>,
        operation: OperationType,
        _context: &mut Context,
    ) {
        let mut tracker = self.tracker.borrow_mut();
        match operation {
            OperationType::Reject => tracker.track_rejection(promise),
            OperationType::Handle => tracker.track_handler(promise),
        }
    }
}

pub(crate) struct BrowserJobExecutor {
    jobs: RefCell<VecDeque<NativeJob>>,
    engine_jobs: Rc<SimpleJobExecutor>,
    engine_jobs_pending: Cell<bool>,
    running: Cell<bool>,
    max_jobs: usize,
    time_slice: Duration,
}

impl BrowserJobExecutor {
    pub(crate) fn new(max_jobs: usize, time_slice: Duration) -> Self {
        Self {
            jobs: RefCell::new(VecDeque::new()),
            engine_jobs: Rc::new(SimpleJobExecutor::new()),
            engine_jobs_pending: Cell::new(false),
            running: Cell::new(false),
            max_jobs: max_jobs.max(1),
            time_slice,
        }
    }

    pub(crate) fn has_pending(&self) -> bool {
        !self.jobs.borrow().is_empty() || self.engine_jobs_pending.get()
    }

    pub(crate) fn pending_counts(&self) -> (usize, bool) {
        (self.jobs.borrow().len(), self.engine_jobs_pending.get())
    }
}

impl JobExecutor for BrowserJobExecutor {
    fn enqueue_job(self: Rc<Self>, job: Job, context: &mut Context) {
        match job {
            Job::PromiseJob(job) => self
                .jobs
                .borrow_mut()
                .push_back(NativeJob::new(move |context| job.call(context))),
            Job::GenericJob(job) => self
                .jobs
                .borrow_mut()
                .push_back(NativeJob::new(move |context| job.call(context))),
            job => {
                self.engine_jobs_pending.set(true);
                self.engine_jobs.clone().enqueue_job(job, context);
            }
        }
    }

    fn run_jobs(self: Rc<Self>, context: &mut Context) -> JsResult<()> {
        if self.running.replace(true) {
            return Ok(());
        }
        let result = (|| {
            if self.engine_jobs_pending.replace(false) {
                if let Err(error) = self.engine_jobs.clone().run_jobs(context) {
                    crate::exceptions::report(error, context);
                }
            }
            let jobs_started = Instant::now();
            for idx in 0..self.max_jobs {
                let next = self.jobs.borrow_mut().pop_front();
                let Some(job) = next else { break };
                if let Err(error) = job.call(context) {
                    crate::exceptions::report(error, context);
                }
                if (idx + 1) % 16 == 0 && jobs_started.elapsed() >= self.time_slice {
                    break;
                }
            }
            Ok(())
        })();
        self.running.set(false);
        result
    }
}
