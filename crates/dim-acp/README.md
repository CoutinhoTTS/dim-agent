# dim-acp

A client-host rutis plugin over the public Dim RPC v3 connection. `AcpGateway`
takes exclusively owned transport halves and an independently connected
`dim_client::Client`; it installs no daemon or Agent parts. The host owns daemon
discovery and startup. `dimcode acp` is the native stdio host.

The implementation contract and capability boundaries are documented in
[`acp-gateway-implementation.md`](../../docs/design/acp-gateway-implementation.md).
Capabilities are advertised only as their handlers become available.
Actual client results and unverified environments are in the
[`acceptance record`](../../docs/design/acp-gateway-acceptance.md).
