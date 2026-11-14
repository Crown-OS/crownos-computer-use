//! Every other toolkit through AT-SPI: GTK, Qt, Electron and Chromium publish their accessible
//! trees on the a11y bus, with window-relative extents and the actions screen readers run.

use std::collections::{HashMap, VecDeque};

use atspi::connection::{read_session_accessibility, set_session_accessibility};
use atspi::proxy::accessible::{AccessibleProxy, ObjectRefExt};
use atspi::proxy::action::ActionProxy;
use atspi::proxy::component::ComponentProxy;
use atspi::proxy::editable_text::EditableTextProxy;
use atspi::{AccessibilityConnection, CoordType, Interface, ObjectRefOwned, State};
use crownos_ui_elements::{Action, Bounds, Element, ElementRef, SnapshotId, Source};
use zbus::names::BusName;
use zbus::proxy::CacheProperties;

use crate::ActResult;
use crate::error::SourceError;
use crate::util::role::snake_case;

const REGISTRY: &str = "org.a11y.atspi.Registry";
const ROOT: &str = "/org/a11y/atspi/accessible/root";
/// Past this a tree is a document, not a UI, and OCR or the DOM serve better.
const NODE_BUDGET: usize = 4_000;
const MAX_DEPTH: u8 = 40;
const CLICK_ACTIONS: &[&str] = &["click", "press", "activate", "jump", "toggle"];

/// The a11y bus and the objects of the last tree read, so refs can be acted on.
pub struct AtspiSource {
    connection: AccessibilityConnection,
    objects: HashMap<String, ObjectRefOwned>,
}

impl std::fmt::Debug for AtspiSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AtspiSource")
            .field("objects", &self.objects.len())
            .finish_non_exhaustive()
    }
}

impl AtspiSource {
    /// Connects to the a11y bus, switching accessibility on for the session if it was off so
    /// toolkits start publishing.
    ///
    /// # Errors
    ///
    /// Fails without an a11y bus.
    pub async fn connect() -> Result<Self, SourceError> {
        if !read_session_accessibility().await.unwrap_or(false) {
            set_session_accessibility(true).await?;
        }
        Ok(Self {
            connection: AccessibilityConnection::new().await?,
            objects: HashMap::new(),
        })
    }

    /// The shown accessibles of the application running as `pid`, breadth first with depth.
    ///
    /// # Errors
    ///
    /// Fails when no application on the bus belongs to `pid`.
    pub async fn elements(&mut self, pid: u32) -> Result<Vec<Element>, SourceError> {
        let connection = self.connection.connection();
        let root = AccessibleProxy::builder(connection)
            .destination(REGISTRY)?
            .path(ROOT)?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        let dbus = zbus::fdo::DBusProxy::new(connection).await?;
        let mut application = None;
        for child in root.get_children().await? {
            let Some(name) = child.name().cloned() else {
                continue;
            };
            let owner = dbus
                .get_connection_unix_process_id(BusName::from(name))
                .await
                .ok();
            if owner == Some(pid) {
                application = Some(child);
                break;
            }
        }
        let application = application.ok_or_else(|| {
            SourceError::Atspi(format!("no accessible application belongs to pid {pid}"))
        })?;
        self.objects.clear();
        let mut elements = Vec::new();
        let mut queue = VecDeque::from([(application, 0u8)]);
        while let Some((object, depth)) = queue.pop_front() {
            if elements.len() >= NODE_BUDGET {
                break;
            }
            let Ok(proxy) = object.as_accessible_proxy(connection).await else {
                continue;
            };
            let state = proxy.get_state().await.unwrap_or_default();
            if depth > 1 && !state.contains(State::Showing) {
                continue;
            }
            let interfaces = proxy.get_interfaces().await.unwrap_or_default();
            let role = snake_case(&proxy.get_role_name().await.unwrap_or_default());
            let bounds = if interfaces.contains(Interface::Component) {
                self.extents(&object).await
            } else {
                Bounds::default()
            };
            let actions = if interfaces.contains(Interface::Action) {
                self.action_names(&object).await
            } else {
                Vec::new()
            };
            let secure = role == "password_text";
            let source_id = source_id(&object);
            elements.push(Element {
                reference: ElementRef {
                    snapshot: SnapshotId(0),
                    index: 0,
                },
                role,
                name: proxy.name().await.unwrap_or_default(),
                value: None,
                bounds,
                actions,
                secure,
                source: Source::Atspi,
                source_id: source_id.clone(),
                depth,
            });
            self.objects.insert(source_id, object.clone());
            if depth < MAX_DEPTH {
                for child in proxy.get_children().await.unwrap_or_default() {
                    queue.push_back((child, depth + 1));
                }
            }
        }
        Ok(elements)
    }

    /// # Errors
    ///
    /// Fails when the element is gone or the toolkit refuses.
    pub async fn act(
        &mut self,
        element: &Element,
        action: &Action,
    ) -> Result<ActResult, SourceError> {
        let object = self
            .objects
            .get(&element.source_id)
            .cloned()
            .ok_or(SourceError::Gone)?;
        let (x, y) = element.bounds.center();
        match action {
            Action::Click => {
                if self.do_action(&object, CLICK_ACTIONS).await? {
                    return Ok(ActResult::Done);
                }
                Ok(ActResult::NeedsPointer { x, y })
            }
            Action::Focus => {
                self.component(&object).await?.grab_focus().await?;
                Ok(ActResult::Done)
            }
            Action::TypeText { text } => {
                let editable = self.editable(&object).await?;
                if let Ok(component) = self.component(&object).await {
                    let _ = component.grab_focus().await;
                }
                let length = i32::try_from(text.chars().count()).unwrap_or(i32::MAX);
                editable.insert_text(i32::MAX, text, length).await?;
                Ok(ActResult::Done)
            }
            Action::SetValue { value } => {
                self.editable(&object)
                    .await?
                    .set_text_contents(value)
                    .await?;
                Ok(ActResult::Done)
            }
            Action::DoubleClick | Action::Hover | Action::Scroll { .. } => {
                Ok(ActResult::NeedsPointer { x, y })
            }
            Action::PressKey { .. } => Err(SourceError::Unsupported("press a key on")),
        }
    }

    async fn extents(&self, object: &ObjectRefOwned) -> Bounds {
        match self.component(object).await {
            Ok(component) => component
                .get_extents(CoordType::Window)
                .await
                .map(|(x, y, width, height)| Bounds::new(x, y, width, height))
                .unwrap_or_default(),
            Err(_) => Bounds::default(),
        }
    }

    async fn action_names(&self, object: &ObjectRefOwned) -> Vec<String> {
        match self.proxy::<ActionProxy<'_>>(object).await {
            Ok(proxy) => proxy
                .get_actions()
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|action| snake_case(&action.name))
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    async fn do_action(
        &self,
        object: &ObjectRefOwned,
        wanted: &[&str],
    ) -> Result<bool, SourceError> {
        let proxy = self.proxy::<ActionProxy<'_>>(object).await?;
        let actions = proxy.get_actions().await?;
        let index = actions
            .iter()
            .position(|action| wanted.contains(&snake_case(&action.name).as_str()));
        match index {
            Some(index) => Ok(proxy
                .do_action(i32::try_from(index).unwrap_or(i32::MAX))
                .await?),
            None => Ok(false),
        }
    }

    async fn component(&self, object: &ObjectRefOwned) -> Result<ComponentProxy<'_>, SourceError> {
        self.proxy(object).await
    }

    async fn editable(
        &self,
        object: &ObjectRefOwned,
    ) -> Result<EditableTextProxy<'_>, SourceError> {
        self.proxy(object).await
    }

    async fn proxy<'a, P>(&'a self, object: &ObjectRefOwned) -> Result<P, SourceError>
    where
        P: zbus::proxy::Defaults + From<zbus::Proxy<'a>>,
    {
        let name = object.name().cloned().ok_or(SourceError::Gone)?;
        let proxy = zbus::proxy::Builder::<'a, P>::new(self.connection.connection())
            .destination(BusName::from(name))?
            .path(object.path().clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        Ok(proxy)
    }
}

fn source_id(object: &ObjectRefOwned) -> String {
    format!(
        "{}{}",
        object.name_as_str().unwrap_or_default(),
        object.path_as_str()
    )
}
