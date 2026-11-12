//! Reads the registry for the contract tests: every error code and frame type. Nothing here is listed by hand.
use crate::api::{ApiError, CommonError, EventInfo, Feature};
use std::collections::BTreeSet;

/// Every error code the Companion can return: the kernel's and each route's.
pub fn error_codes(features: &[Feature]) -> BTreeSet<&'static str> {
    let mut codes: BTreeSet<_> = CommonError::CODES.iter().copied().collect();
    for e in features.iter().flat_map(|f| &f.endpoints) {
        codes.extend(e.errors.iter().copied());
    }
    codes
}
/// Every frame type: the kernel's and each feature's.
pub fn events(features: &[Feature]) -> Vec<EventInfo> {
    let mut events = crate::api::kernel_events();
    for feature in features {
        events.extend(feature.events.iter().map(|e| EventInfo { name: e.name }));
    }
    events
}
