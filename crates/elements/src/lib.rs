//! What an agent sees on the desktop, in one shape.
//!
//! A CrownUI semantic node, an AT-SPI accessible and a DOM node from the browser all become an
//! [`Element`]: a role, a name, a value, bounds in the window and the actions it takes. A
//! [`Snapshot`] is one window's elements at one moment, and every element in it carries a ref
//! such as `@s7:e12` that is only valid for that snapshot, so an agent that acts on a stale view
//! is told so instead of clicking where something else now is.

mod action;
mod element;
mod find;
mod refs;
mod render;
mod snapshot;

pub use action::Action;
pub use element::{Bounds, Element, Source, WindowHandle};
pub use find::Query;
pub use refs::{ElementRef, RefParseError, SnapshotId};
pub use snapshot::{ResolveError, Snapshot, SnapshotStore};
