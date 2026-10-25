//! Dependencies a part's configuration chooses: `When<D>` gates and reads as
//! `D` when its keys are given, and is `None` otherwise ([`Names::when`]).

use rutis::{CordisError, Ctx, Deps, TypeKey};

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
