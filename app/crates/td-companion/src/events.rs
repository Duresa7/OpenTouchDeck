//! Typed changes shared by the native UI, behavioural tests and the web adapter.
use parking_lot::Mutex;
use std::sync::{Arc, Weak};
use tokio::sync::broadcast;

#[derive(Clone)]
pub enum Change {
    DeviceAdded(crate::devices::DeviceAdded),
    DeviceUpdated(crate::devices::DeviceUpdated),
    DeviceRemoved(crate::devices::DeviceRemoved),
    NewBoardAdded(crate::devices::NewBoardAdded),
    NewBoardRemoved(crate::devices::NewBoardRemoved),
    DeviceConsole(crate::devices::DeviceConsole),
    DeviceKeyMirror(crate::devices::DeviceKeyMirror),
    DeviceEvent(crate::devices::DeviceEvent),
    ApprovalRequested(crate::actions::ApprovalRequest),
    ApprovalResolved(crate::actions::ApprovalResolved),
    ActionLogged(crate::actions::ActionLog),
    JobUpdated(crate::jobs::JobUpdated),
    IntegrationStatus(crate::features::integrations::IntegrationStatusEvent),
    Stopping,
}
impl From<crate::devices::DeviceAdded> for Change {
    fn from(value: crate::devices::DeviceAdded) -> Self {
        Self::DeviceAdded(value)
    }
}
impl From<crate::devices::DeviceUpdated> for Change {
    fn from(value: crate::devices::DeviceUpdated) -> Self {
        Self::DeviceUpdated(value)
    }
}
impl From<crate::devices::DeviceRemoved> for Change {
    fn from(value: crate::devices::DeviceRemoved) -> Self {
        Self::DeviceRemoved(value)
    }
}
impl From<crate::devices::NewBoardAdded> for Change {
    fn from(value: crate::devices::NewBoardAdded) -> Self {
        Self::NewBoardAdded(value)
    }
}
impl From<crate::devices::NewBoardRemoved> for Change {
    fn from(value: crate::devices::NewBoardRemoved) -> Self {
        Self::NewBoardRemoved(value)
    }
}
impl From<crate::devices::DeviceConsole> for Change {
    fn from(value: crate::devices::DeviceConsole) -> Self {
        Self::DeviceConsole(value)
    }
}
impl From<crate::devices::DeviceKeyMirror> for Change {
    fn from(value: crate::devices::DeviceKeyMirror) -> Self {
        Self::DeviceKeyMirror(value)
    }
}
impl From<crate::devices::DeviceEvent> for Change {
    fn from(value: crate::devices::DeviceEvent) -> Self {
        Self::DeviceEvent(value)
    }
}
impl From<crate::actions::ApprovalRequest> for Change {
    fn from(value: crate::actions::ApprovalRequest) -> Self {
        Self::ApprovalRequested(value)
    }
}
impl From<crate::actions::ApprovalResolved> for Change {
    fn from(value: crate::actions::ApprovalResolved) -> Self {
        Self::ApprovalResolved(value)
    }
}
impl From<crate::actions::ActionLog> for Change {
    fn from(value: crate::actions::ActionLog) -> Self {
        Self::ActionLogged(value)
    }
}
impl From<crate::jobs::JobUpdated> for Change {
    fn from(value: crate::jobs::JobUpdated) -> Self {
        Self::JobUpdated(value)
    }
}
impl From<crate::features::integrations::IntegrationStatusEvent> for Change {
    fn from(value: crate::features::integrations::IntegrationStatusEvent) -> Self {
        Self::IntegrationStatus(value)
    }
}

/// Synchronous observers are private adapters and must not call back into the event bus.
pub(crate) type Observer = dyn Fn(&Change) + Send + Sync;
#[derive(Clone)]
pub struct Events {
    sender: broadcast::Sender<Change>,
    observers: Arc<Mutex<Vec<Weak<Observer>>>>,
}
impl Default for Events {
    fn default() -> Self {
        Self {
            sender: broadcast::channel(1024).0,
            observers: Default::default(),
        }
    }
}
impl Events {
    pub fn publish<E: Clone + Into<Change>>(&self, event: &E) {
        let change = event.clone().into();
        // Serialize concurrent publishers so native and web observers see the same order.
        let mut observers = self.observers.lock();
        let _ = self.sender.send(change.clone());
        observers.retain(|weak| {
            if let Some(observer) = weak.upgrade() {
                observer(&change);
                true
            } else {
                false
            }
        });
    }
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Change> {
        self.sender.subscribe()
    }
    pub(crate) fn observe(&self, observer: Arc<Observer>) -> Arc<Observer> {
        self.observers.lock().push(Arc::downgrade(&observer));
        observer
    }
}
