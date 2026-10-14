# RemoteGateway

Status: Draft. Overall design: [overview §5.13](../overview.md#513-remotegateway), [§7.2 Transport, Connection](../overview.md#72-app-server-process-plugins), [§3.1 Multiple nodes](../overview.md#31-multiple-nodes), [common constraint 8](../overview.md#3-integration-methods). Phase: P3 (authentication, disconnection and TLS trade-offs with [#5](https://github.com/arcships/dim/issues/5)). Language: Rust.

RemoteGateway opens an authenticated entry on Transport's WebSocket for phones and the Web workspace: pairing by QR code or pairing code, per-device tokens with rotation and revocation, and the Web workspace static assets. Remote clients speak the existing App Server protocol and get Session methods only; Management is not mounted on this entry. The entry is the product UI plane; it never carries the rutis link plane between nodes.

## 1. User flows

1. `/remote` → start: RemoteGateway persists `enabled` and opens its entry (loopback by default). `remote.status` shows the address.
2. `/remote` → pair: the CLI calls `remote.pair` and renders the returned URL as a QR code with the pairing code. The phone opens the URL, submits the code with a device name, and receives its device token. `remote.devices` lists the phone.
3. The phone opens the Web workspace, connects with its token, lists Sessions, attaches to the Session the terminal is attached to, and sees the same live events. Either side submits input, answers approvals or stops the turn.
4. The phone locks and the socket drops. The Session keeps running; a pending approval stays pending. On reconnect the phone attaches again and receives the pending approval by replay; the terminal may answer it first.
5. The user revokes the phone (`remote.revoke`): its connections close at once; its token no longer authenticates.
6. The user forces token rotation (`remote.rotate_token`): devices are disconnected and must rotate their token before reconnecting; a copied old token stops working once rotated.

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| Entry lifecycle (start, stop, status) and its persisted intent | Session behavior, events, approvals (existing owners through Rpc Session methods) |
| Pairing offers, device records, device tokens, rotation, revocation | Node-to-node authentication and service export (rutis link, #5) |
| Authenticating WebSocket upgrades and attaching device identity to connections | Management authorization (mounting only; see [Management](Management.md)) |
| Serving Web workspace static assets | Building the Web workspace; rendering QR codes (CLI) |
| Management methods `remote.*` | TLS certificate issuance; reverse proxies and tunnels |

## 3. Scope and dependencies

**Contracts defined here** (owner: RemoteGateway): device records and tokens, the pairing and rotation HTTP endpoints, `remote.*` management methods, the `remote-devices.json` host-local format. **Contracts depended on**: Transport's entry port and upgrade-authenticator contract (Transport change, proposed below; Transport is the consumer of authenticators and defines that contract), Connection's per-connection entry record (Connection change), Rpc's Session-method table and its entry mount rule ([Management §6.4](Management.md#64-rpc-change-and-mounting)), `FileStorage` leases, typed Settings operations for its own configuration (#3), rutis-loader volatile configuration updates (`x-volatile`, `VolatileUpdate`, verified in rutis-loader 0.10), the [Management contribution contract](Management.md#61-contribution-contract). RemoteGateway knows no Session method's meaning and no other plugin.

| Part | Scope | Injects | Provides | Status |
| --- | --- | --- | --- | --- |
| `RemoteGatewayPlugin` | Process (control node) | `EntryHost` (Transport), `FileStorage`, `SettingsService` | `ManagementContribution` (`remote`) | Proposed |
| `TransportPlugin` | Process | existing | existing `ListenAddresses`; added `EntryHost` | Change |
| `ConnectionPlugin` / `RpcPlugin` | Process | existing | Connection records `ConnectionEntry`; Rpc gates Management and `daemon.shutdown` by `mounts_management` | Change |

Transport change (proposed contract, owned by Transport):

```rust
impl EntryHost {
    /// Binds a listener served by Transport's HTTP/WebSocket stack; dropping the
    /// handle closes the listener and every connection it accepted (detach).
    pub async fn open(&self, spec: EntrySpec) -> Result<EntryHandle, EntryError>;
}
pub struct EntrySpec {
    pub name: String,                              // "remote"
    pub bind: SocketAddr,
    pub tls: Option<TlsFiles>,                     // cert and key paths on the control node
    pub authenticate: Arc<dyn UpgradeAuthenticator>,
    pub routes: Arc<dyn HttpRoutes>,               // non-WebSocket paths (assets, pairing)
    pub mounts_management: bool,                   // assembly data; RemoteGateway passes false (A1)
}
pub trait UpgradeAuthenticator: Send + Sync {
    /// Called once per WebSocket upgrade with the request headers.
    fn authenticate(&self, request: &UpgradeRequest) -> HostFuture<'_, Result<Authenticated, Rejected>>;
}
pub struct Authenticated { pub device_id: String, pub revoked: CancellationToken }
pub struct ConnectionEntry { pub name: String, pub mounts_management: bool, pub device_id: Option<String> }
```

The connection closes when `revoked` fires. Without RemoteGateway, Transport's existing bare WebSocket keeps today's rules (protected profiles: `hello` and `daemon.ping` only).

## 4. Events

None. Devices, tokens and offers are host-local security material and never enter Timeline (overview §9). Remote clients produce Session facts only through existing Session methods (`session.submit` → `input/message`, `approval.respond` → `approval/respond`, `session.stop` → `stop/interrupt`), written by those owners after their boundary validation; the device that caused them is not recorded in Timeline (A2).

## 5. State

| State | Owner, storage | Rebuild |
| --- | --- | --- |
| Entry intent: `enabled`, `bind`, `tls`, `assets` | RemoteGateway configuration in the loader editable layer through Settings; `enabled`, `bind`, `tls` are `x-volatile` | Persisted (#3) |
| Device records `{device_id, name, token_hash, previous_hash?, previous_until?, rotation_required, paired_at, last_seen_at}` | `remote-devices.json`, version 1, private file through `FileStorage` atomic replacement (A3) | Persisted; corrupt or unreadable file fails closed (no device authenticates) |
| Pairing offers `{code_hash, expires_at, attempts}` | RemoteGateway memory | Not persisted; lost on restart (A4) |
| Live connections per device and their revocation tokens | RemoteGateway memory | Empty after reload or restart |
| Session attachments, pending approval replay | Connection, Approval (existing) | Existing |

## 6. Interfaces

### 6.1 Entry and HTTP surface

| Path | Auth | Behavior |
| --- | --- | --- |
| `GET /`, `GET /assets/*` | None | Web workspace static files from the distribution's assets directory; confined to that directory; immutable cache headers; restrictive Content-Security-Policy (A5) |
| `POST /api/pair` | Pairing code | Body `{code, deviceName, web?}`. Valid unexpired offer → new device record; returns `{deviceId, token}`; with `web: true` sets the token as an `HttpOnly; Secure; SameSite=Strict` cookie instead of returning it (A6). Offers are single use |
| `POST /api/token/rotate` | Current or previous token | Returns a new token; the previous one stays valid for this endpoint and upgrades until the new token is first used or 10 minutes pass (A7) |
| `GET /ws` | `Authorization: Bearer <token>` or the cookie with a same-origin `Origin` header | Upgrade to the App Server protocol; rejected with 401 when the token is unknown, revoked or `rotation_required` |

Tokens are 32 random bytes, base64url; only SHA-256 hashes are stored and compared in constant time. Pairing codes are 8 characters of Crockford base32 (A8). Failed pairing attempts are limited to 5 per offer and 20 per minute per entry; exceeding the limit invalidates the offer (A9). Loopback is the default bind; a non-loopback bind requires `tls` or an explicit `plaintext: true` (A10).

### 6.2 Protocol on the remote entry

| Item | Rule |
| --- | --- |
| Version | `hello` with protocol v3 only; v1/v2 are rejected with `protocol_version` (A11) |
| Methods | Rpc Session methods ([Management §6.5](Management.md#65-classification-and-migration-of-existing-methods)) on every Session of this host (A12); management names and `daemon.shutdown` → `-32601 unknown_method` |
| `hello.features` | Session features only; no management names, no `management.methods` |
| Identity | `ConnectionEntry.device_id` is attached to the connection; Rpc uses it for diagnostics and for closing on revocation; it grants nothing beyond the entry's method set |
| Reverse requests | None: client requests, server notifies ([#5](https://github.com/arcships/dim/issues/5)) |

### 6.3 Management methods (namespace `remote`)

| Method | Params | Result | Interruption |
| --- | --- | --- | --- |
| `remote.start` | `{bind?, tls?, plaintext?}` | `{address}` after the listener is bound | Report |
| `remote.stop` | `{}` | `{}` after the listener and its connections closed | Report |
| `remote.status` | `{}` | `{enabled, running, address?, tls, error?, devices, connections}` | Drain |
| `remote.pair` | `{ttlSeconds?}` (default 300, max 900) | `{code, url, expiresAt}`; `url` is `<origin>/pair#code=<code>` | Drain |
| `remote.devices` | `{}` | `[{deviceId, name, pairedAt, lastSeenAt, connections, rotationRequired}]` | Drain |
| `remote.revoke` | `{deviceId}` | `{}` after the record is removed and its connections closed | Report |
| `remote.rotate_token` | `{deviceId?}` (absent: all devices) | `{}` after the flag is persisted and affected connections closed | Report |

`remote.start` and `remote.stop` write `enabled` (and `bind`/`tls`) through the typed Settings operation for this plugin's configuration; the loader delivers the volatile change as `VolatileUpdate` without reloading the plugin, and RemoteGateway opens or drops its `EntryHandle`. Restart is `stop` then `start` (CLI). CLI entries: `/remote`, `/web`, `dimcode remote`, `dimcode web`; running the daemon in the foreground or background is the CLI's concern.

## 7. Timeline and capability mechanics

| Step | Path |
| --- | --- |
| Writes | None by RemoteGateway. Remote calls reach Session owners through Rpc exactly like local calls: `session.submit` → `Session::input` (boundary validation, `Timeline::append`); `approval.respond` → branch Approval service, which appends `approval/respond`; `session.stop` → turn `stop/interrupt` |
| Dispatcher | Not subscribed. Remote clients receive events through their per-connection Session attachment (existing `session_channel` listener), not through the process broadcast subscription |
| Concurrent attachers | Each connection holds its own attachment; all attachers receive the same dispatched events in per-branch order; there is no exclusive owner of a Session (A13) |
| Recheck | Decisions belong to the Session owners and are made by sequence: the first `approval/respond` consumed by Approval decides; later responses fail `invalid_params` ("absent or already resolved") |
| Reload | RemoteGateway reload drops its `EntryHandle`: listener and remote connections close (detach); device records persist; offers are lost. Transport reload reloads RemoteGateway (it injects `EntryHost`) |
| Process restart | The entry reopens if `enabled`; devices reconnect and re-attach; committed inputs and pending approvals are unaffected and replayed on attach |

## 8. Timing and conflicts

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Phone and terminal answer the same approval | first `approval/respond` < second | First decides; second gets `invalid_params`; v2/v3 attachers see the deciding event | Approval |
| Remote answer after a turn `stop/interrupt` | interrupt < response | Request already `approval/cancelled`; response fails | Approval |
| Remote answer before the interrupt | response < interrupt | Decision accepted; tool may still be stopped before start | Approval, ToolExecutor |
| Branch-scoped interrupt or branch close while a remote client is attached | — | Client receives the events; later calls on that branch fail `session_invalid_input`/closed | Session owners |
| Phone Steer while terminal Queue input pending | Timeline order | Applied per mode in sequence order | InputQueue, Loop |
| Remote submit during an unsettled tool batch | input < `step/tool-result` | Steer/Append after the batch result; Queue after `turn/end` | Loop |
| Compaction or child branch cut around a remote input | — | Same as any input ([session access §8](../contracts/session-access.md#8-timing-and-conflicts)) | ContextPreparation, InputQueue |
| Question answered from two devices | first `question/answered` | First accepted by sequence (overview §5.1) | Questions |
| Disconnect while an approval is pending | — | Stays pending; replayed on re-attach; no timeout (A14) | Approval |
| Disconnect after `session.submit` admitted, reply lost | input committed | Reconnect, `session.input_lookup` by `submissionId` | Client |
| Revocation while a request of that device runs | admitted < revoke | Connection closed; admitted command continues; reply lost | Rpc |
| Two phones use one pairing code | first `POST /api/pair` | First pairs; second fails, counts as an attempt | RemoteGateway |
| Rotation response lost | — | Previous token still accepted during the grace window; device retries rotation | RemoteGateway |
| `remote.rotate_token` while a device rotates | persisted flag order | A rotation committed before the flag is also flagged; device rotates again | RemoteGateway |
| `remote.stop` with attached devices | — | Connections close (detach); Sessions continue | RemoteGateway, Transport |
| Rpc reload | — | Remote calls get `service_reloading`; connections stay unless the protocol instance changes | Rpc |
| Session closed by another client | `session.closed` | Remote attachers receive `session.closed` | Connection |
| Session idle unload while only remote attachers exist | — | Attachment pins the Session as today; detach releases | AgentIndex |
| Resource node of a Session offline | capabilities Pending | Remote requests get the same retryable errors as local ones | rutis lifecycle |
| Process restart with offers outstanding | — | Offers lost; devices persist | RemoteGateway |

## 9. Context mapping

No model-visible effect of its own. Content from remote clients enters the model exactly as local client content: `session.submit` user content (text, images) becomes one user message at the input's consumed position; it carries no source marker because the user is its author; approvals and stops are not model content. Compaction, child-branch inheritance and non-interactive Sessions follow the Session input rules ([session access §9](../contracts/session-access.md#9-context-mapping)). The model never sees device ids, device names, tokens, entry names or diagnostics.

## 10. Errors and recovery

| Case | Result |
| --- | --- |
| Bind failure on `remote.start` | `-32000 remote_bind_failed`; `enabled` stays false |
| TLS files unreadable | `-32000 remote_tls_invalid`; not started |
| Non-loopback bind without TLS or `plaintext` | `-32602 invalid_params` |
| Device file corrupt or unreadable | `remote.status.error`; all upgrades rejected; `remote.devices` fails `remote_storage_failed`; file kept for repair |
| Unknown device id | `-32602 invalid_params` |
| Upgrade with bad token | HTTP 401, no detail; counted in diagnostics without the token |
| Listener fails while running | Entry closed, `remote.status.error` set, `enabled` unchanged; the daemon keeps running (A15) |

## 11. Security

The remote entry grants full Session-level control of every Session on the host to every paired device, including answering approvals, `session.permission.update` and creating Sessions in any authorized directory (A12); it never grants management: pairing, revocation, configuration, approvals of host-local content and shutdown require an entry that mounts Management. Mounting Management on the remote entry is an assembly option and equals giving every paired device host administrator rights; it is not a management method. Authentication stays minimal (overview §3.1): one bearer token per device, hashed at rest, rotated by the device, revoked locally; pairing codes are short-lived, single-use and rate-limited. Cookies are `HttpOnly`, `Secure`, `SameSite=Strict` and require a same-origin `Origin`. Tokens never appear in URLs, logs, diagnostics or Timeline. Static assets are public and contain no data. The entry never accepts rutis link frames or exports services; node-to-node authority uses rutis link tokens on its own plane (#5).

## 12. Acceptance

Flow tests in the App Server suite over a real WebSocket entry:

1. `remote.start`/`status`/`stop`; non-loopback without TLS rejected; restart reopens when `enabled`.
2. Pairing with a valid code, an expired code, a reused code and too many attempts; `remote.devices` lists the device.
3. A paired device calls Session methods; every management method, `management.methods` and `daemon.shutdown` return `unknown_method`; `hello.features` contains no management names; v1 is rejected.
4. Two attached clients: both receive events; competing `approval.respond` — one success, one `invalid_params`; interrupt before and after a remote response.
5. Disconnect with a pending approval and with a submitted input; reconnect, re-attach receives the replay; `session.input_lookup` finds the input.
6. Revoke closes the device's connections and rejects its token; rotation keeps the old token through the grace window and then rejects it; `remote.rotate_token` forces rotation.
7. A corrupt device file rejects all upgrades without resetting the file.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | The mount flag is entry assembly data; default false for RemoteGateway | Overview common constraint 8 | — |
| A2 | Device identity is not written to Timeline | Overview §9 (security settings); judgment | Inputs would carry a device attribution field or marker |
| A3 | Devices stored in a private host-local JSON file via FileStorage | Precedent `mcp-approvals.json`; judgment | SQLite table with a format version change |
| A4 | Pairing offers are in memory only | Judgment | Persisted offers survive restart |
| A5 | Web workspace assets ship with the distribution and are served unauthenticated | Feature list `/web`; judgment | Assets require authentication |
| A6 | Browser pairing uses an HttpOnly cookie; native clients receive the token | Judgment | Token in browser storage |
| A7 | Device-initiated rotation with a 10-minute grace window; local forced rotation by flag | Feature list "rotate access tokens"; judgment | Different rotation model |
| A8 | Pairing code format and 32-byte tokens | Judgment | — |
| A9 | Attempt limits 5 per offer and 20 per minute | Judgment | Different limits |
| A10 | Loopback default; non-loopback needs TLS or explicit plaintext | Overview §5.13 (TLS with #5); judgment | Deployment policy decided in #5 |
| A11 | Remote entry accepts protocol v3 only | Judgment (new clients) | v1/v2 compatibility on the entry |
| A12 | A paired device can access every Session of the host | Feature list "access projects and Sessions"; overview §7.2 | Per-device workspace scoping |
| A13 | No exclusive Session ownership; "take over" means attaching and acting | Feature list "collaborate with or take over" | A control-lock protocol |
| A14 | Pending approvals have no timeout when no client is attached | Existing Approval behavior | Timeout policy in Approval |
| A15 | A listener failure stops only the remote entry | Judgment; today a listener failure stops the daemon | Daemon shutdown on remote failure |

## 14. Open questions

| Question | Description |
| --- | --- |
| [#5](https://github.com/arcships/dim/issues/5) and overview §13 "Remote access" | Relation of product device identity to peer tokens, link disconnection, TLS deployment |
| Session methods with authority | Whether `session.permission.update` and `session.create` with arbitrary `cwd` should be excluded from the remote entry; this would be the first per-entry method filter beyond mounting |
| Per-device scope | Whether devices may be limited to workspaces or Sessions |
| Overview §7.2 Transport/Connection | The `EntryHost` and authenticator contracts are proposed here and need the Transport change document |
