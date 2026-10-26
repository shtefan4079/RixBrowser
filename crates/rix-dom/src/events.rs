
use crate::tree::Document;
use indextree::NodeId;
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPhase {
    None,
    Capturing,
    AtTarget,
    Bubbling,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub event_type: String,
    pub target: Option<NodeId>,
    pub current_target: Option<NodeId>,
    pub phase: EventPhase,
    pub bubbles: bool,
    pub cancelable: bool,
    propagation_stopped: Arc<AtomicBool>,
    default_prevented: Arc<AtomicBool>,
}

impl Event {
    pub fn new(event_type: impl Into<String>, bubbles: bool, cancelable: bool) -> Self {
        Self {
            event_type: event_type.into(),
            target: None,
            current_target: None,
            phase: EventPhase::None,
            bubbles,
            cancelable,
            propagation_stopped: Arc::new(AtomicBool::new(false)),
            default_prevented: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn stop_propagation(&self) {
        self.propagation_stopped.store(true, Ordering::SeqCst);
    }

    pub fn is_propagation_stopped(&self) -> bool {
        self.propagation_stopped.load(Ordering::SeqCst)
    }

    pub fn prevent_default(&self) {
        if self.cancelable {
            self.default_prevented.store(true, Ordering::SeqCst);
        }
    }

    pub fn is_default_prevented(&self) -> bool {
        self.default_prevented.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseButton {
    #[default]
    Primary = 0,
    Auxiliary = 1,
    Secondary = 2,
    Back = 3,
    Forward = 4,
}

#[derive(Debug, Clone)]
pub struct MouseEvent {
    pub base: Event,
    pub client_x: f32,
    pub client_y: f32,
    pub screen_x: f32,
    pub screen_y: f32,
    pub button: MouseButton,
    pub buttons: u16,
    pub alt_key: bool,
    pub ctrl_key: bool,
    pub shift_key: bool,
    pub meta_key: bool,
}

impl MouseEvent {
    pub fn new(
        event_type: impl Into<String>,
        client_x: f32,
        client_y: f32,
        button: MouseButton,
    ) -> Self {
        Self {
            base: Event::new(event_type, true, true),
            client_x,
            client_y,
            screen_x: client_x,
            screen_y: client_y,
            button,
            buttons: 1,
            alt_key: false,
            ctrl_key: false,
            shift_key: false,
            meta_key: false,
        }
    }
}

impl Deref for MouseEvent {
    type Target = Event;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone)]
pub struct KeyboardEvent {
    pub base: Event,
    pub key: String,
    pub code: String,
    pub repeat: bool,
    pub alt_key: bool,
    pub ctrl_key: bool,
    pub shift_key: bool,
    pub meta_key: bool,
}

impl KeyboardEvent {
    pub fn new(
        event_type: impl Into<String>,
        key: impl Into<String>,
        code: impl Into<String>,
    ) -> Self {
        Self {
            base: Event::new(event_type, true, true),
            key: key.into(),
            code: code.into(),
            repeat: false,
            alt_key: false,
            ctrl_key: false,
            shift_key: false,
            meta_key: false,
        }
    }
}

impl Deref for KeyboardEvent {
    type Target = Event;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone)]
pub struct FocusEvent {
    pub base: Event,
    pub related_target: Option<NodeId>,
}

impl FocusEvent {
    pub fn new(event_type: impl Into<String>, related_target: Option<NodeId>) -> Self {
        Self {
            base: Event::new(event_type, false, false),
            related_target,
        }
    }
}

impl Deref for FocusEvent {
    type Target = Event;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[derive(Debug, Clone)]
pub struct InputEvent {
    pub base: Event,
    pub data: Option<String>,
    pub input_type: String,
    pub is_composing: bool,
}

impl InputEvent {
    pub fn new(
        event_type: impl Into<String>,
        data: Option<String>,
        input_type: impl Into<String>,
    ) -> Self {
        Self {
            base: Event::new(event_type, true, false),
            data,
            input_type: input_type.into(),
            is_composing: false,
        }
    }
}

impl Deref for InputEvent {
    type Target = Event;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

pub type EventCallback = Arc<dyn Fn(&Event) + Send + Sync>;

#[derive(Clone)]
struct RegisteredListener {
    callback: EventCallback,
    use_capture: bool,
}

#[derive(Default)]
pub struct EventDispatcher {
    listeners: HashMap<NodeId, HashMap<String, Vec<RegisteredListener>>>,
}

impl EventDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_event_listener(
        &mut self,
        node: NodeId,
        event_type: &str,
        callback: EventCallback,
        use_capture: bool,
    ) {
        let node_listeners = self.listeners.entry(node).or_default();
        let list = node_listeners.entry(event_type.to_string()).or_default();
        list.push(RegisteredListener {
            callback,
            use_capture,
        });
    }

    pub fn dispatch(&self, document: &Document, target: NodeId, mut event: Event) -> Event {
        event.target = Some(target);

        let mut capture_path: Vec<NodeId> = document.ancestors(target).skip(1).collect();
        capture_path.reverse();

        event.phase = EventPhase::Capturing;
        for &node in &capture_path {
            if event.is_propagation_stopped() {
                return event;
            }
            event.current_target = Some(node);
            self.invoke_listeners(node, &event, true);
        }

        if !event.is_propagation_stopped() {
            event.phase = EventPhase::AtTarget;
            event.current_target = Some(target);
            self.invoke_listeners(target, &event, true);
            self.invoke_listeners(target, &event, false);
        }

        if event.bubbles && !event.is_propagation_stopped() {
            event.phase = EventPhase::Bubbling;
            for &node in capture_path.iter().rev() {
                if event.is_propagation_stopped() {
                    return event;
                }
                event.current_target = Some(node);
                self.invoke_listeners(node, &event, false);
            }
        }

        event
    }

    fn invoke_listeners(&self, node: NodeId, event: &Event, is_capture: bool) {
        if let Some(node_listeners) = self.listeners.get(&node) {
            if let Some(list) = node_listeners.get(&event.event_type) {
                for listener in list {
                    if listener.use_capture == is_capture {
                        (listener.callback)(event);
                    }
                }
            }
        }
    }
}
