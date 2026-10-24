//! Linux (UPD-4).

use crate::stage::Staged;
use std::path::Path;

pub(super) fn after_check(_app: suite_core::AppKind, _updates: &Path, _staged: &Staged) {}
