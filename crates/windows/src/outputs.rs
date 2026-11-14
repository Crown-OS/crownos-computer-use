use std::collections::HashMap;

use wayland_client::globals::{GlobalList, GlobalListContents};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Dispatch, QueueHandle};

const OUTPUT_VERSION: u32 = 4;

#[derive(Debug, Clone)]
pub(crate) struct TrackedOutput {
    pub(crate) output: WlOutput,
    pub(crate) name: Option<String>,
}

/// Every wl_output, bound as it appears.
#[derive(Debug, Default)]
pub(crate) struct Outputs {
    by_global: HashMap<u32, TrackedOutput>,
}

impl Outputs {
    pub(crate) fn bind_existing<S>(globals: &GlobalList, queue: &QueueHandle<S>) -> Self
    where
        S: Dispatch<WlOutput, u32> + 'static,
    {
        let mut outputs = Self::default();
        globals.contents().with_list(|list| {
            for global in list.iter().filter(|global| global.interface == "wl_output") {
                let output = globals.registry().bind::<WlOutput, u32, S>(
                    global.name,
                    global.version.min(OUTPUT_VERSION),
                    queue,
                    global.name,
                );
                outputs.insert(global.name, output);
            }
        });
        outputs
    }

    fn insert(&mut self, global: u32, output: WlOutput) {
        self.by_global
            .insert(global, TrackedOutput { output, name: None });
    }

    /// The output called `name`, or any when `None` or unknown.
    pub(crate) fn find(&self, name: Option<&str>) -> Option<&TrackedOutput> {
        name.and_then(|name| {
            self.by_global
                .values()
                .find(|output| output.name.as_deref() == Some(name))
        })
        .or_else(|| self.by_global.values().next())
    }

    pub(crate) fn on_registry_event<S>(
        &mut self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        queue: &QueueHandle<S>,
    ) where
        S: Dispatch<WlOutput, u32> + 'static,
    {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "wl_output" => {
                let output = registry.bind::<WlOutput, u32, S>(
                    name,
                    version.min(OUTPUT_VERSION),
                    queue,
                    name,
                );
                self.insert(name, output);
            }
            wl_registry::Event::GlobalRemove { name } => {
                self.by_global.remove(&name);
            }
            _ => {}
        }
    }

    pub(crate) fn on_output_event(&mut self, global: u32, event: wl_output::Event) {
        let Some(output) = self.by_global.get_mut(&global) else {
            return;
        };
        if let wl_output::Event::Name { name } = event {
            output.name = Some(name);
        }
    }
}

impl<S> Dispatch<WlRegistry, GlobalListContents, S> for Outputs
where
    S: Dispatch<WlRegistry, GlobalListContents> + 'static,
{
    fn event(
        _state: &mut S,
        _registry: &WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _connection: &wayland_client::Connection,
        _queue: &QueueHandle<S>,
    ) {
    }
}
