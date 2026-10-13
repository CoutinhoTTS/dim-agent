//! Typed capability dependencies: a capability names its dependencies once,
//! as a rutis [`Deps`] type, and the scope it was built for gives their
//! runtime keys. Reads delegate to rutis, so a dependency lost after the
//! gate opened returns the plugin to Pending as for any typed plugin.

use crate::*;
use rutis::{CordisError, Ctx, DepKey, Deps, Gate, Keyed, KeyedGate, TypeKey, TypedPlugin};
use std::{marker::PhantomData, ops::Deref, sync::Arc};

/// The runtime keys of a typed plugin's dependencies.
pub type KeysOf<P> = <<P as TypedPlugin>::Deps as Deps>::Keys;

/// A typed plugin that knows the runtime keys of its dependencies, usually
/// from the scope it was built for: `scoped::<Self::Deps, _>(&self.scope)`.
pub trait Capability: TypedPlugin {
    fn keys(&self) -> KeysOf<Self>;
}

/// Dependencies whose keys follow from a scope `S`. A Session scope cannot
/// give branch keys, so a Session capability declaring [`InBranch`] does not
/// compile.
pub trait ScopedDeps<S>: Deps {
    fn scoped(scope: &S) -> Self::Keys;
}

/// The keys of `D` in `scope`.
pub fn scoped<D: ScopedDeps<S>, S>(scope: &S) -> D::Keys {
    D::scoped(scope)
}

/// A capability as a rutis plugin gated on its declared keys, for mounting
/// it directly (process capabilities, tests).
pub fn capability_plugin<P: Capability>(plugin: P) -> rutis::Typed<P> {
    let keys = plugin.keys();
    rutis::Typed::with_keys(plugin, keys)
}

pub(crate) fn typed<P: Capability>(plugin: P) -> Box<dyn rutis::Plugin> {
    Box::new(capability_plugin(plugin))
}

/// Registers a capability as a loader module named after the plugin and
/// returns the name. `make` must be pure: registration constructs and drops
/// one plugin to read its metadata, and each load constructs another. Every
/// plugin must have the same name and keys. Capture shared configuration in
/// `make` when it must survive a reload; acquire I/O resources only in apply.
pub fn register_capability<P: Capability>(
    builtins: &mut rutis_loader::Builtins,
    make: impl Fn() -> P + Send + Sync + 'static,
) -> String {
    let first = make();
    let name = first.name().to_owned();
    let keys = first.keys();
    builtins.register::<NoConfig, _>(
        name.clone(),
        rutis::TypedFactory::with_keys(
            Make {
                name: name.clone(),
                make,
            },
            keys,
        ),
    );
    name
}
struct Make<F> {
    name: String,
    make: F,
}
impl<P: Capability, F: Fn() -> P + Send + Sync + 'static> rutis::TypedPluginFactory<NoConfig>
    for Make<F>
{
    type Plugin = P;
    fn name(&self) -> &str {
        &self.name
    }
    fn build(&self, _: &NoConfig) -> std::result::Result<P, CordisError> {
        Ok((self.make)())
    }
}

fn session_key<T: ?Sized + 'static>(scope: &SessionScope) -> DepKey<T> {
    DepKey::of().instance(scope.instance)
}
fn branch_key<T: ?Sized + 'static>(scope: &BranchScope) -> DepKey<T> {
    DepKey::of().instance(scope.instance)
}

/// A service of the capability's Session.
pub struct InSession<T: ?Sized>(pub Arc<T>);
impl<T: ?Sized> Deref for InSession<T> {
    type Target = Arc<T>;
    fn deref(&self) -> &Arc<T> {
        &self.0
    }
}
impl<T: ?Sized + SessionService> Deps for InSession<T> {
    type Keys = SessionScope;
    fn injects(scope: &SessionScope, out: &mut Vec<TypeKey>) {
        out.push(scope.key::<T>());
    }
    fn resolve(scope: &SessionScope, ctx: &Ctx) -> std::result::Result<Self, CordisError> {
        Keyed::<T>::resolve(&session_key(scope), ctx).map(|service| Self(service.0))
    }
}

/// A service of the capability's branch.
pub struct InBranch<T: ?Sized>(pub Arc<T>);
impl<T: ?Sized> Deref for InBranch<T> {
    type Target = Arc<T>;
    fn deref(&self) -> &Arc<T> {
        &self.0
    }
}
impl<T: ?Sized + BranchService> Deps for InBranch<T> {
    type Keys = BranchScope;
    fn injects(scope: &BranchScope, out: &mut Vec<TypeKey>) {
        out.push(scope.key::<T>());
    }
    fn resolve(scope: &BranchScope, ctx: &Ctx) -> std::result::Result<Self, CordisError> {
        Keyed::<T>::resolve(&branch_key(scope), ctx).map(|service| Self(service.0))
    }
}

/// Gates the capability on a Session service without reading it.
pub struct SessionGate<T: ?Sized>(PhantomData<fn() -> T>);
impl<T: ?Sized + SessionService> Deps for SessionGate<T> {
    type Keys = SessionScope;
    fn injects(scope: &SessionScope, out: &mut Vec<TypeKey>) {
        out.push(scope.key::<T>());
    }
    fn resolve(scope: &SessionScope, ctx: &Ctx) -> std::result::Result<Self, CordisError> {
        KeyedGate::<T>::resolve(&session_key(scope), ctx).map(|_| Self(PhantomData))
    }
}

/// Gates the capability on a branch service without reading it.
pub struct BranchGate<T: ?Sized>(PhantomData<fn() -> T>);
impl<T: ?Sized + BranchService> Deps for BranchGate<T> {
    type Keys = BranchScope;
    fn injects(scope: &BranchScope, out: &mut Vec<TypeKey>) {
        out.push(scope.key::<T>());
    }
    fn resolve(scope: &BranchScope, ctx: &Ctx) -> std::result::Result<Self, CordisError> {
        KeyedGate::<T>::resolve(&branch_key(scope), ctx).map(|_| Self(PhantomData))
    }
}

/// Every named service of a list in the capability's Session, in order.
pub struct Contributions<T: ?Sized>(pub Vec<(String, Arc<T>)>);
impl<T: ?Sized> Deref for Contributions<T> {
    type Target = Vec<(String, Arc<T>)>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T: ?Sized + SessionService> Deps for Contributions<T> {
    type Keys = (SessionScope, Arc<[String]>);
    fn injects((scope, names): &Self::Keys, out: &mut Vec<TypeKey>) {
        out.extend(names.iter().map(|name| scope.named_key::<T>(name)));
    }
    fn resolve((scope, names): &Self::Keys, ctx: &Ctx) -> std::result::Result<Self, CordisError> {
        names
            .iter()
            .map(|name| {
                let key = DepKey::<T>::dynamic(name.as_str()).instance(scope.instance);
                Keyed::<T>::resolve(&key, ctx).map(|service| (name.clone(), service.0))
            })
            .collect::<std::result::Result<_, _>>()
            .map(Self)
    }
}

/// A dependency the configuration enables or not, fixed when the plugin is
/// built: when enabled it gates and reads as `D`, otherwise it is `None`.
pub struct When<D>(pub Option<D>);
impl<D: Deps> Deps for When<D> {
    type Keys = Option<D::Keys>;
    fn injects(keys: &Self::Keys, out: &mut Vec<TypeKey>) {
        if let Some(keys) = keys {
            D::injects(keys, out);
        }
    }
    fn resolve(keys: &Self::Keys, ctx: &Ctx) -> std::result::Result<Self, CordisError> {
        keys.as_ref()
            .map(|keys| D::resolve(keys, ctx))
            .transpose()
            .map(Self)
    }
}

impl<T: ?Sized + Send + Sync + 'static, S> ScopedDeps<S> for Arc<T> {
    fn scoped(_: &S) {}
}
impl<T: ?Sized + 'static, S> ScopedDeps<S> for Gate<T> {
    fn scoped(_: &S) {}
}
impl<S> ScopedDeps<S> for () {
    fn scoped(_: &S) {}
}
impl<T: ?Sized + SessionService> ScopedDeps<SessionScope> for InSession<T> {
    fn scoped(scope: &SessionScope) -> SessionScope {
        *scope
    }
}
impl<T: ?Sized + SessionService> ScopedDeps<BranchScope> for InSession<T> {
    fn scoped(scope: &BranchScope) -> SessionScope {
        scope.session
    }
}
impl<T: ?Sized + SessionService> ScopedDeps<SessionScope> for SessionGate<T> {
    fn scoped(scope: &SessionScope) -> SessionScope {
        *scope
    }
}
impl<T: ?Sized + SessionService> ScopedDeps<BranchScope> for SessionGate<T> {
    fn scoped(scope: &BranchScope) -> SessionScope {
        scope.session
    }
}
impl<T: ?Sized + BranchService> ScopedDeps<BranchScope> for InBranch<T> {
    fn scoped(scope: &BranchScope) -> BranchScope {
        *scope
    }
}
impl<T: ?Sized + BranchService> ScopedDeps<BranchScope> for BranchGate<T> {
    fn scoped(scope: &BranchScope) -> BranchScope {
        *scope
    }
}

macro_rules! tuple_scoped {
    ($($name:ident),+) => {
        impl<Scope, $($name: ScopedDeps<Scope>),+> ScopedDeps<Scope> for ($($name,)+) {
            fn scoped(scope: &Scope) -> Self::Keys {
                ($($name::scoped(scope),)+)
            }
        }
    };
}
tuple_scoped!(A);
tuple_scoped!(A, B);
tuple_scoped!(A, B, C);
tuple_scoped!(A, B, C, D);
tuple_scoped!(A, B, C, D, E);
tuple_scoped!(A, B, C, D, E, F);
tuple_scoped!(A, B, C, D, E, F, G);
tuple_scoped!(A, B, C, D, E, F, G, H);
