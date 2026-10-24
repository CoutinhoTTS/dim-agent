//! Services by name ([How Dim uses rutis §3](../../../docs/design/rutis-usage.md#3-services-by-name)).
//!
//! A service is a name in one group: the process, [`SESSION_GROUP`] or
//! [`BRANCH_GROUP`]. The catalog holds each name once with its group. A Rust
//! part is a plain rutis [`TypedPlugin`] whose dependencies are [`Keyed`]
//! services: the Rust binding takes a service's name from its type, as
//! `#[rutis::service]` does (`TreeCommands` is `treeCommands`), its group from
//! the catalog, and its key for each copy from the instances the copy runs in.
//! A part therefore names no group: where its row is placed decides, and a
//! row placed outside a group one of its names belongs to fails at
//! composition, naming that group.

use crate::*;
use rutis::{CordisError, DepKey, Deps, InstanceId, Keyed, KeyedGate, Plugin, TypedPlugin};
use rutis_loader::{Build, LoaderError, ServiceCatalog};
use std::any::{TypeId, type_name};
use std::result::Result;

/// The service name of the Rust type `T`: its type name with the first word
/// lowercased, as `#[rutis::service]` names a type (`Timeline` is
/// `timeline`, `dyn LoopState` `loopState`, `HTTPClient` `httpClient`).
pub fn name_of<T: ?Sized>() -> String {
    let full = type_name::<T>();
    let full = full.strip_prefix("dyn ").unwrap_or(full);
    let path = full.split(['<', ' ']).next().unwrap_or(full);
    let ident = path.rsplit("::").next().unwrap_or(path);
    let chars: Vec<char> = ident.chars().collect();
    let upper = chars.iter().take_while(|c| c.is_uppercase()).count();
    let lower = if upper > 1 && upper < chars.len() && chars[upper].is_lowercase() {
        upper - 1
    } else {
        upper.max(1).min(chars.len())
    };
    chars
        .iter()
        .enumerate()
        .flat_map(|(index, c)| {
            if index < lower {
                c.to_lowercase().collect::<Vec<_>>()
            } else {
                vec![*c]
            }
        })
        .collect()
}

/// Registers the name of `T` in `group`, or at the process for `None`.
/// Registering it again in the same group does nothing, so each crate's
/// list of names can be applied by every installer that needs it.
///
/// # Panics
///
/// When the name is already registered in another group: a name belongs to
/// one group.
pub fn register<T: ?Sized + Send + Sync + 'static>(
    catalog: &mut ServiceCatalog,
    group: Option<&str>,
) {
    let name = name_of::<T>();
    let known = match catalog.key_in(&name, &Build::default()) {
        Err(LoaderError::UnknownService(_)) => None,
        Ok(_) => Some(None),
        Err(LoaderError::OutsideInstance { group, .. }) => Some(Some(group)),
        Err(error) => panic!("{name}: {error}"),
    };
    match known {
        None => match group {
            None => catalog.register::<T>(name),
            Some(group) => catalog.register_instance::<T>(name, group),
        },
        Some(known) => {
            assert_eq!(
                known.as_deref(),
                group,
                "the service {name} is registered in two groups"
            );
            catalog
        }
    };
}

/// A catalog for an assembly: it shares only the names registered as shared
/// (`share_registered_only`), and holds the runtime's own names.
pub fn catalog() -> ServiceCatalog {
    let mut catalog = ServiceCatalog::new();
    catalog.share_registered_only();
    let session = Some(SESSION_GROUP);
    register::<SessionConfig>(&mut catalog, session);
    register::<AgentConfig>(&mut catalog, session);
    register::<SessionIdentity>(&mut catalog, session);
    register::<SessionLifecycle>(&mut catalog, session);
    register::<FaultReporter>(&mut catalog, session);
    register::<TimelineFactory>(&mut catalog, session);
    register::<dyn TimelineStore>(&mut catalog, session);
    register::<dyn TimelineTreeControl>(&mut catalog, session);
    register::<SessionLoader>(&mut catalog, session);
    let branch = Some(BRANCH_GROUP);
    register::<Timeline>(&mut catalog, branch);
    register::<Dispatcher>(&mut catalog, branch);
    register::<TimelineIdentity>(&mut catalog, branch);
    register::<BranchLifecycle>(&mut catalog, branch);
    register::<BranchOpening>(&mut catalog, branch);
    register::<dyn LoopState>(&mut catalog, branch);
    register::<dyn ConfigureControl>(&mut catalog, branch);
    catalog
}

/// Where one copy of a part runs, to find the keys of the names it depends
/// on.
pub struct Names<'a> {
    catalog: &'a ServiceCatalog,
    place: Place<'a>,
}
enum Place<'a> {
    /// A copy running in these instances.
    Copy(&'a Build),
    /// The composition check of a part placed inside `groups`; keys use a
    /// placeholder instance.
    Check {
        groups: &'a [&'a str],
        instance: Option<InstanceId>,
    },
}
impl<'a> Names<'a> {
    /// The names of a copy running in `build`'s instances.
    pub fn copy(catalog: &'a ServiceCatalog, build: &'a Build) -> Self {
        Self {
            catalog,
            place: Place::Copy(build),
        }
    }
    /// The composition check of a part placed inside `groups`, innermost
    /// first; `instance` stands in for each of them (none at the process).
    pub fn check(
        catalog: &'a ServiceCatalog,
        groups: &'a [&'a str],
        instance: Option<InstanceId>,
    ) -> Self {
        Self {
            catalog,
            place: Place::Check { groups, instance },
        }
    }
    /// The instance of the enclosing `group` the copy runs in.
    pub fn instance(&self, group: &str) -> Result<InstanceId, CordisError> {
        match &self.place {
            Place::Copy(build) => build.instance(group),
            Place::Check { groups, instance } if groups.contains(&group) => {
                instance.ok_or_else(|| outside(group))
            }
            Place::Check { .. } => Err(outside(group)),
        }
    }
    /// The key of `T`'s name for this copy.
    pub fn key<T: ?Sized + 'static>(&self) -> Result<DepKey<T>, CordisError> {
        let name = name_of::<T>();
        let failed = |message: String| CordisError::PluginFailed(message.into());
        let group = match self.catalog.key_in(&name, &Build::default()) {
            Ok(key) => {
                self.check_type::<T>(&name, &key)?;
                return Ok(DepKey::of());
            }
            Err(LoaderError::OutsideInstance { group, .. }) => group,
            Err(LoaderError::UnknownService(_)) => {
                return Err(failed(format!("{name} is not a registered service")));
            }
            Err(error) => return Err(failed(error.to_string())),
        };
        let instance = match &self.place {
            Place::Copy(build) => {
                let key = self
                    .catalog
                    .key_in(&name, build)
                    .map_err(|_| misplaced(&name, &group))?;
                self.check_type::<T>(&name, &key)?;
                build.instance(&group)?
            }
            Place::Check { groups, instance } => {
                if !groups.contains(&group.as_str()) {
                    return Err(misplaced(&name, &group));
                }
                instance.ok_or_else(|| misplaced(&name, &group))?
            }
        };
        Ok(DepKey::of().instance(instance))
    }
    /// The keys of the dependencies `D` for this copy.
    pub fn keys<D: NamedDeps>(&self) -> Result<D::Keys, CordisError> {
        D::keys(self)
    }
    /// The keys of a dependency the part's configuration enables or not.
    pub fn when<D: NamedDeps>(&self, enabled: bool) -> Result<Option<D::Keys>, CordisError> {
        enabled.then(|| D::keys(self)).transpose()
    }
    /// The Rust part `plugin`, gated on its dependencies' keys for this copy.
    pub fn part<P>(&self, plugin: P) -> Result<Box<dyn Plugin>, CordisError>
    where
        P: TypedPlugin,
        P::Deps: NamedDeps,
    {
        let keys = self.keys::<P::Deps>()?;
        Ok(Box::new(rutis::Typed::with_keys(plugin, keys)))
    }
    /// The Rust part `plugin` with keys its factory computed, for
    /// configuration-chosen dependencies ([`Self::when`]).
    pub fn part_with<P: TypedPlugin>(
        &self,
        plugin: P,
        keys: <P::Deps as Deps>::Keys,
    ) -> Box<dyn Plugin> {
        Box::new(rutis::Typed::with_keys(plugin, keys))
    }
    /// A name registered for a Rust service of another type is a programming
    /// error; a shared name crosses languages and has no Rust type here.
    fn check_type<T: ?Sized + 'static>(
        &self,
        name: &str,
        key: &rutis::TypeKey,
    ) -> Result<(), CordisError> {
        if self.catalog.is_shared(name) || key.type_id() == TypeId::of::<T>() {
            return Ok(());
        }
        Err(CordisError::PluginFailed(
            format!(
                "{name} is a {}, not a {}",
                key.type_name(),
                type_name::<T>()
            )
            .into(),
        ))
    }
}
fn outside(group: &str) -> CordisError {
    CordisError::PluginFailed(format!("the part is not inside a {group} instance").into())
}
fn misplaced(name: &str, group: &str) -> CordisError {
    CordisError::PluginFailed(
        format!("{name} is a {group} service: place the part inside the {group} group").into(),
    )
}

/// Dependencies whose keys follow from their names: [`Keyed`], [`KeyedGate`],
/// `Option<Keyed<T>>` (read if present, without gating), `()` and tuples of
/// up to eight.
pub trait NamedDeps: Deps {
    fn keys(names: &Names<'_>) -> Result<Self::Keys, CordisError>;
}
impl<T: ?Sized + Send + Sync + 'static> NamedDeps for Keyed<T> {
    fn keys(names: &Names<'_>) -> Result<DepKey<T>, CordisError> {
        names.key::<T>()
    }
}
impl<T: ?Sized + Send + Sync + 'static> NamedDeps for Option<Keyed<T>> {
    fn keys(names: &Names<'_>) -> Result<DepKey<T>, CordisError> {
        names.key::<T>()
    }
}
impl<T: ?Sized + 'static> NamedDeps for KeyedGate<T> {
    fn keys(names: &Names<'_>) -> Result<DepKey<T>, CordisError> {
        names.key::<T>()
    }
}
impl NamedDeps for () {
    fn keys(_: &Names<'_>) -> Result<(), CordisError> {
        Ok(())
    }
}
macro_rules! named_tuple {
    ($($name:ident),+) => {
        impl<$($name: NamedDeps),+> NamedDeps for ($($name,)+) {
            fn keys(names: &Names<'_>) -> Result<Self::Keys, CordisError> {
                Ok(($($name::keys(names)?,)+))
            }
        }
    };
}
named_tuple!(A);
named_tuple!(A, B);
named_tuple!(A, B, C);
named_tuple!(A, B, C, D);
named_tuple!(A, B, C, D, E);
named_tuple!(A, B, C, D, E, F);
named_tuple!(A, B, C, D, E, F, G);
named_tuple!(A, B, C, D, E, F, G, H);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_service_macro() {
        assert_eq!(name_of::<Timeline>(), "timeline");
        assert_eq!(name_of::<TimelineIdentity>(), "timelineIdentity");
        assert_eq!(name_of::<dyn LoopState>(), "loopState");
        assert_eq!(name_of::<dyn TimelineStore>(), "timelineStore");
        assert_eq!(name_of::<StateCell<u8>>(), "stateCell");
        struct HTTPClient;
        assert_eq!(name_of::<HTTPClient>(), "httpClient");
    }
}
