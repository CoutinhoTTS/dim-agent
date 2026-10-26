import { type Plugin, definePlugin } from '@arcships/rutis'
import type { PartRegistry, PartScope } from '../../npm/plugin.js'

// A fixture part as a plugin's one row (rutis usage §2): the row registers
// `part` for `scope` with `partRegistry`, and each Session or branch mounts
// a copy of it, which closes over the row's config.
export function partRow<C>(scope: PartScope, part: Plugin<C>) {
  return definePlugin<C>({
    name: part.name,
    inject: ['partRegistry'],
    async apply(ctx, config) {
      const copy = definePlugin({ ...part, apply: (at, _) => part.apply(at, config) })
      await ctx.use<PartRegistry>('partRegistry').registerDefinition(scope, part.name ?? 'part', copy)
    },
  })
}
