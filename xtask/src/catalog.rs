//! The service catalog table ([How Dim uses rutis](../../docs/design/rutis-usage.md)
//! §3.4 and R4).
//!
//! `graph` serves the App Server product profile, sharing the names `serve`
//! shares when it hosts foreign parts (`session_typescript::share_names`), opens
//! one Session and writes `generated/catalog.md` from that assembly: every
//! catalog name one of its parts provides or injects, with its group, provider,
//! whether it crosses languages and the methods its dispatcher serves, and the
//! parts that are Capabilities because they inject `dispatcher` or `timeline`.
//! `check` builds the same assembly and fails when the file differs.
use crate::app_server;
use dim_agent_runtime::{Feature, SessionConfig, names::name_of_type};
use rutis::{
    BoxFuture, CordisError, Ctx, Effect, InstanceId, RuntimeDiagnostics, TypeKey, TypedPlugin,
};
use rutis_bridge::session::{HostDispatch, host_key};
use rutis_loader::{Build, LoaderError, ServiceCatalog};
use std::{
    any::TypeId,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The names whose injection makes a part a Capability (plugin contract §6).
const CAPABILITY_NAMES: [&str; 2] = ["dispatcher", "timeline"];
/// The plugins rutis serves an exported Rust service through: not Dim parts,
/// and not its provider, which provides the Rust service they serve.
const EXPORT_PREFIX: &str = "rutis-export:";
/// Groups in the order the table lists them.
const GROUPS: [&str; 3] = ["process", "session", "branch"];

fn file(root: &Path) -> PathBuf {
    root.join("docs/architecture/rust-agent/generated/catalog.md")
}

/// Writes the catalog table of the product profile.
pub async fn write(root: &Path) -> Result<()> {
    std::fs::write(file(root), collect().await?)?;
    Ok(())
}

/// Fails when the written catalog table is not the product profile's.
pub async fn check(root: &Path) -> Result<()> {
    let path = file(root);
    let written = std::fs::read_to_string(&path).unwrap_or_default();
    if written != collect().await? {
        return Err(format!(
            "{}: catalog table is stale; run `cargo run -p xtask -- graph`",
            path.display()
        )
        .into());
    }
    println!("service catalog passed");
    Ok(())
}

/// A branch part that keeps its context, to read the dispatchers the
/// assembly serves names with. It names and provides nothing.
#[derive(Clone, Default)]
struct Probe(Arc<Mutex<Option<Ctx>>>);
impl TypedPlugin for Probe {
    type Deps = ();
    fn name(&self) -> &str {
        "CatalogProbe"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): (),
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            *self.0.lock().unwrap() = Some(ctx.clone());
            Ok(Effect::Done)
        })
    }
}

/// The catalog table of the product profile with one Session.
async fn collect() -> Result<String> {
    let dir = app_server::scratch("catalog")?;
    let probe = Probe::default();
    let server = app_server::serve_product(&dir, |builder| {
        dim_app_server::session_typescript::share_names(builder.catalog_mut());
        let probe = probe.clone();
        builder.require_feature(
            Feature::new("CatalogProbe")
                .branch("CatalogProbe", move |names| names.part(probe.clone())),
        );
    })
    .await?;
    let table = async {
        let agent = server
            .agent()
            .ok_or("the product profile has no default Agent")?;
        // No mount list: the Session declares every tool. A list chooses
        // among tools, not parts.
        let session = agent
            .create_session(SessionConfig {
                cwd: dir.to_string_lossy().into_owned(),
                tools: None,
                ..Default::default()
            })
            .await?;
        let layers = [
            ("process", app_server::process_layer(server.diagnostics())),
            ("session", session.diagnostics()?),
            ("branch", session.branch_diagnostics("main")?),
        ];
        let ctx = probe.0.lock().unwrap().take();
        let ctx = ctx.ok_or("the catalog probe did not load")?;
        let table = Table::new(agent.catalog(), &layers, &ctx)?.render();
        session.close().await?;
        Ok::<_, Box<dyn std::error::Error>>(table)
    }
    .await;
    server.shutdown();
    server.join().await?;
    let _ = std::fs::remove_dir_all(&dir);
    table
}

#[derive(Default)]
struct Service {
    group: &'static str,
    providers: BTreeSet<String>,
    crosses: bool,
    methods: BTreeMap<String, String>,
}

struct Capability {
    group: &'static str,
    part: String,
    injects: BTreeSet<String>,
}

struct Table {
    services: BTreeMap<String, Service>,
    capabilities: Vec<Capability>,
}

impl Table {
    fn new(
        catalog: &ServiceCatalog,
        layers: &[(&'static str, RuntimeDiagnostics)],
        ctx: &Ctx,
    ) -> Result<Self> {
        // Instance keys carry the instance of their scope plugin's group.
        let mut instances = BTreeMap::new();
        for (group, scope) in [("session", "SessionScope"), ("branch", "BranchScope")] {
            let (_, layer) = layers
                .iter()
                .find(|(layer, _)| *layer == group)
                .ok_or("missing layer")?;
            let scope = layer
                .plugins
                .iter()
                .find(|p| p.name == scope)
                .ok_or("missing scope plugin")?;
            let instance = layer
                .plugins
                .iter()
                .find(|p| Some(p.id) == scope.parent)
                .map(|p| p.instance)
                .ok_or("missing scope group")?;
            instances.insert(group, instance);
        }
        let names = Names {
            catalog,
            instances: &instances,
        };
        let plugins: BTreeMap<_, _> = layers
            .iter()
            .flat_map(|(_, layer)| &layer.plugins)
            .map(|plugin| (plugin.id, plugin.name.as_str()))
            .collect();
        let mut services = BTreeMap::<String, Service>::new();
        let mut capabilities = Vec::new();
        for (layer, diagnostics) in layers {
            for binding in &diagnostics.bindings {
                let Some(found) = names.of(&binding.key) else {
                    continue;
                };
                let provider = plugins.get(&binding.provider).copied().unwrap_or("?");
                let service = services.entry(found.name).or_default();
                service.group = found.group;
                if !provider.starts_with(EXPORT_PREFIX) {
                    service.providers.insert(provider.to_owned());
                }
            }
            for plugin in &diagnostics.plugins {
                if plugin.name.starts_with(EXPORT_PREFIX) {
                    continue;
                }
                let mut injects = BTreeSet::new();
                for inject in &plugin.injects {
                    if let Some(found) = names.of(&inject.key) {
                        services.entry(found.name.clone()).or_default().group = found.group;
                        if CAPABILITY_NAMES.contains(&found.name.as_str()) {
                            injects.insert(found.name);
                        }
                    }
                }
                if !injects.is_empty() {
                    capabilities.push(Capability {
                        group: layer,
                        part: plugin.name.clone(),
                        injects,
                    });
                }
            }
        }
        for (name, service) in &mut services {
            service.crosses = catalog.is_shared(name);
            if service.crosses {
                let key = match instances.get(service.group) {
                    Some(instance) => host_key(name).with_instance(*instance),
                    None => host_key(name),
                };
                if let Some(methods) = ctx
                    .get_as::<dyn HostDispatch>(key)
                    .and_then(|dispatch| dispatch.methods())
                {
                    service.methods = serde_json::from_value(methods)?;
                }
            }
        }
        capabilities.sort_by(|a, b| (order(a.group), &a.part).cmp(&(order(b.group), &b.part)));
        Ok(Self {
            services,
            capabilities,
        })
    }

    fn render(&self) -> String {
        let mut out = String::from(
            "# Service catalog\n\n<!-- Generated by `cargo run -p xtask -- graph`; do not edit. -->\n\n\
Generated from the real assembly: the App Server product profile (`serve` with a product home, sharing the names it shares when it hosts foreign parts) with one Session and its `main` branch. \
`cargo run -p xtask -- check` fails when this file is stale. \
The rules for names are in [How Dim uses rutis §3](../../../design/rutis-usage.md#3-services-by-name).\n\n\
## Services\n\n\
Each catalog name a part of the assembly provides or injects. A name that crosses languages lists the methods its dispatcher serves, each `sync` or `async` (callers in other languages always get a Promise); other methods of its Rust type are Rust-only. \
A Rust-only name has no dispatcher: Rust parts reach it at its type's key.\n\n\
| Name | Group | Provider | Crosses languages | Methods |\n| --- | --- | --- | --- | --- |\n",
        );
        let mut rows: Vec<_> = self.services.iter().collect();
        rows.sort_by_key(|(name, service)| (order(service.group), *name));
        for (name, service) in rows {
            let provider = if service.providers.is_empty() {
                "—".to_owned()
            } else {
                service
                    .providers
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let crosses = if service.crosses { "yes" } else { "Rust-only" };
            let methods = if service.methods.is_empty() {
                "—".to_owned()
            } else {
                service
                    .methods
                    .iter()
                    .map(|(method, mode)| format!("`{method}` {mode}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            out += &format!(
                "| `{name}` | {} | {provider} | {crosses} | {methods} |\n",
                service.group
            );
        }
        out += "\n## Capabilities\n\n\
The parts that inject `dispatcher` or `timeline` ([plugin contract §6](../../../design/plugin-contract.md#6-capability)). \
Every other part is not a Capability.\n\n\
| Part | Group | Injects |\n| --- | --- | --- |\n";
        for capability in &self.capabilities {
            let injects = capability
                .injects
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ");
            out += &format!(
                "| {} | {} | {injects} |\n",
                capability.part, capability.group
            );
        }
        out
    }
}

fn order(group: &str) -> usize {
    GROUPS
        .iter()
        .position(|known| *known == group)
        .unwrap_or(GROUPS.len())
}

/// Which catalog name a key of the assembly is.
struct Names<'a> {
    catalog: &'a ServiceCatalog,
    /// The Session's and the branch's instance.
    instances: &'a BTreeMap<&'static str, InstanceId>,
}
struct Found {
    name: String,
    group: &'static str,
}
impl Names<'_> {
    /// A dispatcher's key carries its name; a Rust key is named after its
    /// type, as the Rust binding names it. The catalog must hold that name,
    /// in the group whose instance the key is qualified with.
    fn of(&self, key: &TypeKey) -> Option<Found> {
        let dispatcher = key.type_id() == TypeId::of::<dyn HostDispatch>();
        let name = match key.name() {
            Some(name) if dispatcher => name.to_owned(),
            None if !dispatcher => name_of_type(key.type_name()),
            _ => return None,
        };
        let group = match self.catalog.key_in(&name, &Build::default()) {
            Ok(registered) => {
                let rust = &registered == key;
                let shared = registered == host_key(&name);
                (key.instance_id().is_none() && (rust || shared)).then_some("process")?
            }
            Err(LoaderError::OutsideInstance { group, .. }) => {
                let (group, instance) = self.instances.get_key_value(group.as_str())?;
                (key.instance_id() == Some(*instance)).then_some(*group)?
            }
            Err(_) => return None,
        };
        Some(Found { name, group })
    }
}
