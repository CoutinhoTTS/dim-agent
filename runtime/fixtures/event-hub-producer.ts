import { definePlugin } from '@arcships/rutis'
import type { EventHub, EventHubErrorThrown, EventHubPublishRequest } from '../../npm/plugin.js'

const source = 'tsproducer'
const type = 'tsproducer.ping'

/**
 * A producer row of another language: it injects `eventHub`, registers its
 * `eventhub/ready` listener before it reads the service, declares its source,
 * and publishes one event per notice. A publish that finds the declaration
 * missing declares again and retries with the same id, because delivery is
 * idempotent per branch.
 */
export default definePlugin({
  name: 'EventHubProducerFixture',
  inject: ['eventHub'],
  provides: {},
  apply(ctx) {
    let published = 0
    const declare = (hub: EventHub) => hub.declare({
      source,
      guide: 'The TypeScript fixture source: `tsproducer.ping` says the producer works.',
      types: [{ type, description: 'A ping from the TypeScript fixture', mode: 'queue' }],
    })
    const publish = async () => {
      const hub = ctx.use<EventHub>('eventHub')
      await declare(hub)
      const request: EventHubPublishRequest = {
        source,
        type,
        id: `ts-${++published}`,
        body: 'hello from TypeScript',
      }
      try {
        await hub.publish(request)
      } catch (error) {
        const failure = error as EventHubErrorThrown
        if (failure.code !== 'unknown_source') throw failure
        await declare(hub)
        await hub.publish(request)
      }
    }
    // The listener comes first, so a notice from a later generation is not
    // missed; the injected name is readable now, so this row declares too
    // (design 2.1: at its own apply or on that notice).
    ctx.on('eventhub/ready', publish)
    return publish()
  },
})
